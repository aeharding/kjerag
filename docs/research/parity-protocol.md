# The Studio parity protocol, pre-registered

Written 2026-08-08 on `research/parity-proof`, **before the exports it governs
exist**. Nothing in it may be relaxed after the owner's data arrives; a
criterion chosen once the answer is visible is not a criterion.

Everything below was proved on the two exports that already existed
(`~/Videos/Insta/studio_exports/{tiny-planet,20-fov-creek}.mp4`, July-14, made
with Stitching Optimization **ON**). Those two can prove the machinery and can
settle nothing about the reading: their optical flow warp is in the picture, so
a residual measured against them is Studio's flow plus Studio's calibration
plus ours, and no arithmetic separates the three.

## 1. The question, and what an answer would look like

The X4 Air's `offset_v6` string has been read sixteen ways and every internal
candidate has been refused by the owner's eye against Studio
(`docs/research/offset-v6.md`). So the reading is to be recovered from the
outside: Studio renders a static reframe of a file we also hold, and there is
some calibration that makes our renderer draw that same picture. **Find it by
solving for it.**

A positive answer is a calibration, plus a picture drawn with it that the
owner cannot tell from Studio's. A negative answer is a residual that will not
come down, reported with its structure.

## 2. What the pipeline is, stage by stage, and the gate on each

| stage | instrument | gate |
| --- | --- | --- |
| pair the export to the source in TIME | `scripts/research/pair.py` | G4 below |
| aim our renderer at their view | `--bin seam mode=solve aim=1` | G5 |
| solve the calibration | `--bin seam mode=solve` | G1, G2, G3, G6, G7 |
| show the owner | `mode=solve out=<dir>` | section 6 |

### Pairing

`source_time = export_time + lag`, `lag` from `pair.py`. Measured
2026-08-08 on both July-14 exports: `lag = -0.02298 s`, identical to five
decimals on both, standard deviation `0.00000 s` over eight disjoint 40 s
windows spanning 30 to 1700 s, and a fitted drift of `-0.0000 s` across the
whole 1799.8 s clip.

A residual of a few tens of milliseconds is the size of an AAC priming delay
and audio alone cannot tell one from a real offset. **The pairing is therefore
established to within one frame and quoted that way.** Anything finer is the
picture's job, not the audio's.

### Aiming

Studio's pan/tilt frame is not ours and the map between them has never been
derived, so the view's three angles are **solved**, not converted. So is the
view's field of view: Studio's FOV number is not the half angle our projection
family measures in, on either export (recorded in `ab097f8` on
`research/oracle-probe`, and reproduced here — see section 7).

## 3. THE MATCH CRITERION, fixed now

A run **matches** if and only if all three hold.

**(a) Absolute.** The solved residual is **≤ 1.0 px rms at the export's own
pixel scale**, over **≥ 100 kept sites** of which **≥ 40 lie within 8 degrees
of the seam**, spanning **≥ 60 degrees of seam azimuth**, at **both** view
geometries the owner exports.

**(b) Discriminating.** The same measurement, made with the shipped factory
calibration in place of the solved one, is **at least 3x worse**. If it is
not, the run has not discriminated between calibrations and the answer is
"no discrimination", which is not a match.

**(c) Gated.** Every plant gate in section 4 passes **in the same run, on the
same frames**.

Anything else is **NO MATCH**, and is reported as a failure with the residual's
structure: the per-azimuth residual profile and its first three harmonics,
which name the error (constant along the seam is relative roll, one cycle is
the principal point, two cycles is the focal aspect — `--bin seam`'s own
header derives this).

### Why 1.0 px

- **It is 25 to 75 times the arithmetic floor.** The estimator's noise floor,
  measured by solving against an unperturbed render of our own, is 0.013 to
  0.041 px rms across every geometry tried (0.0134 px at the geometry section 5
  asks the owner for). All the headroom between that and 1.0 px is for things
  that are not calibration: Studio's resampling, their tone curve and
  sharpening, HEVC, and their reframe's own filtering.
- **It is 30 times finer than the thing being decided.** At a 60 degree view on
  a 1920-wide export, 1.0 px is 0.031 degrees of world angle. The candidate
  readings in `docs/research/offset-v6.md` differ from each other by 0.76 to
  1.19 degrees along the seam. A criterion at 0.031 degrees separates them
  with two orders of magnitude to spare.
- **It is at the edge of what the eye can be shown.** Below about a pixel at
  delivery resolution a doubled edge stops being visible, which is the
  standard the owner's eye actually applies.

It is deliberately **not** set at the arithmetic floor. A criterion the real
world cannot reach is a criterion that reports failure forever.

## 4. The plant gates, per analysis run

These run on the **same build, same frames, same view geometry** as the
reported answer. A run that reports a calibration without them beside it is
not evidence.

- **G1 NULL.** Solve against an unperturbed render of our own at that geometry
  and instant. Residual **≤ 0.06 px rms**, and every lens-1 angle within
  **0.08 deg** of zero. (Measured 2026-08-08 over three geometries: residual
  0.0134 to 0.0332 px, worst lens-1 angle 0.046 deg.)
- **G2 PLANT.** Solve against our own render with **`cx +8 px, pitch +0.2 deg`**
  on lens 1, started from zero with the view **1.2, -0.9, 0.4 deg** and the
  field of view **3 deg** off truth. Recover lens 1's three angles to
  **≤ 0.10 deg** and the residual to the G1 floor. Measured: pitch came back
  0.2022, 0.1988 and 0.1438 at the three geometries, and the residual came back
  to 0.0126, 0.0331 and 0.0405 px.
  **Read the plant against its own null, not against zero.** Each geometry
  carries a small standing offset that is a property of the geometry and not of
  the plant, and it appears identically in both runs. Null-referenced, the
  planted `cx +8` came back as **7.913**, **8.018** and 6.495 px and the planted
  `pitch +0.2 deg` as **0.1994**, **0.1996** and 0.1225 deg. The third geometry
  is the one with only 40 sites near the seam, which is what section 5's aim
  requirement exists to avoid.
- **G3 REFUSAL.** The same solve at a view carrying one lens only must print
  `REFUSED`. This proves the refusal path is live in the build that produced
  the answer, rather than assumed to be.
- **G4 TIME.** `pair.py`'s four controls pass on the new export: injected
  +/-1 and +/-4 frame shifts recovered to **< 0.05 frames**; the peak beats its
  own +/-1 frame neighbours by **> 0.02** correlation on every window; the lag's
  spread across windows is **< 0.5 frames**; the fitted drift across the clip is
  **< 1 frame**.
- **G5 AIM.** The registration ladder (`aim=1`) is monotone away from the
  fitted pose, and the fitted pose beats **+/-2 deg** in every axis by a margin
  larger than the ladder's own step at 0.25 deg.
- **G6 CONDITIONING.** The printed 5x5 correlation matrix accompanies every
  reported knob. **Any pair past 0.99 is reported as a combination and never as
  two numbers.** At every geometry measured so far, `lens1 yaw` against
  `lens1 cx` runs to -0.97 or worse, so this gate always bites: the five knobs
  are a family, not a point, and the answer is the picture plus the family.
- **G7 CAPTURE.** `search` must exceed the aim uncertainty in degrees times the
  working picture's pixels per degree, and the run must report more sites in
  round 1 than in round 0. A site whose true shift is past the search is
  dropped, not found, so an under-sized search silently starves the fit: at
  `search=12` on a 60 degree view 640 px wide, a start 1.2 deg off kept 50 of
  500 sites and refused, and the same start at `search=30` solved to 0.013 px.

## 5. What the owner must export, and why

Each item is here because it was measured, not because it sounded prudent.

1. **Stitching Optimization OFF.** Already planned. With it on, their optical
   flow is in the picture and the residual stops being about calibration.
2. **The seam must be in the middle of the frame.** The solve separates the
   view's own pose from lens 1's correction only because a view rotation moves
   both lenses' content and a lens 1 correction moves one lens's. A view
   carrying one lens cannot do it and the instrument refuses.
3. **Field of view 50 to 70, not 20.** Measured 2026-08-08: at 20 degrees wide
   in our projection, **every** aim from yaw 40 to 130 refuses — below 90 the
   whole view is lens 0, above it the whole view is lens 1, and at 90 the whole
   view is inside the blend band, so no site is drawn by one lens alone. At 60
   degrees, aims from yaw 70 to 100 all answer, with 91 to 103 sites within 8
   degrees of the seam.
4. **Distortion 0.** A told Panini `d` works — the identity at `d=0` and the
   inverse are both tested in `--bin seam`'s own tests — but it adds a scale
   unknown to a solve that already has one.
5. **Two aims, not one.** Section 4's G6: at any single view, `lens1 yaw` and
   `lens1 cx` are one number wearing two names. Two aims at different points
   of the seam ring is the cheapest thing that can break that, and it costs
   one more export.
6. **1 to 2 minutes each is right**, and the reason is section 7's second
   finding: single frames fail often, for reasons of content rather than
   geometry, and the analysis has to pool the frames that pass.

## 6. What the owner is shown

Never a number on its own. `mode=solve out=<dir>` writes, at the owner's own
judged view and at the export's own resolution:

- `theirs.png` — Studio's frame, as the pipeline read it;
- `ours.png` — our render with the solved calibration;
- `difference-4x.png` — the difference, amplified 4x about mid grey.

The claim put to him is "these two are the same picture, and here is where they
are not". His eye is the acceptance test; the residual is only what decides
whether he should be asked to look.

## 7. What this pipeline cannot do, stated before it is asked to

- **It cannot report five independent knobs.** Section 4's G6. It reports a
  picture and a family.
- **It cannot use the July-14 exports for the reading.** Optimization was on.
- **It cannot find a scale it is more than about 15 degrees away from.**
  Measured: starting the field of view 6 deg off recovers it to 0.012 deg, 15
  deg off to 0.050 deg; **30 deg off breaks** (residual 2.63 px against a floor
  of 0.026, on 23 sites instead of 535) and 60 deg off refuses. Since Studio's
  FOV label is about 2.2 to 2.4x away from our family's number on both existing
  exports, **a coarse scale search must precede the solve** and its result must
  be shown to have a peak.
- **It cannot promise any given frame will solve.** Of eight runs at the
  recommended geometry across four instants, several refused or diverged
  because of what was in the picture — a hemisphere of sky has nothing to
  correlate. Every one of those failures announced itself: `REFUSED`, or a
  residual 10 to 100 times the floor with the site count collapsed. **No
  failure returned a small residual.** That property, not the success rate, is
  what makes the pipeline safe to point at real data.

## 8. The blocker that stands between this and the owner's export

**The coarse aim-and-scale search has not been built.** The solve is local: it
needs to start within about 15 degrees of the true scale and within a search
radius of the true aim. On the two existing exports nothing gets it there.

- `mode=parity` fitted the creek export at 0.74 correlation with its field of
  view walking to 48.6 for a slider that said 20, and its pitch to -79 for a
  tilt that said 3.5. Its ladder is not a peak.
- On the tiny planet it reached 0.52 with the field of view **saturated against
  the top of its own search** at 328 for a slider that said 150.
- The registration ladder run against the real creek export at the parity pose
  is **flat within +/- 1 degree** (0.7366 at the pose, 0.7380 at 0.5 deg off)
  and only falls by 4 degrees out. There is no peak there to refine.

Both existing exports put Studio's FOV label about 2.2 to 2.4x away from our
family's number, which is a testable hypothesis (that their slider is a half
angle) and is not yet tested. Until a coarse search demonstrably peaks on a
real Studio frame, a solve pointed at the owner's new exports would be starting
outside its own capture radius, and section 7 records exactly what that looks
like: at 30 degrees of scale error the solve returns a residual 100x the floor
on 4 percent of the sites, and past that it refuses. It would not lie. It would
simply not answer, and the owner's ten minutes would buy nothing.

## 9. The proving run

`scripts/research/pair.py`, `scripts/research/plants.sh`, and
`--bin seam mode=solve` on branch `research/parity-proof`. Outputs land in
gitignored `scratch/`: they are frames of somebody's real flights and this
repo is public.
