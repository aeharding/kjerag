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
| find their view from cold | `--bin seam mode=register` | R1 to R7, section 4a |
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

Studio's pan/tilt frame is not ours, so the view's three angles are **found**,
not converted, and so are its scale and its projection. `mode=register` is what
finds them, and section 8 is what it had to get past. What it reports has been
measured against both existing exports and is written down in section 4b: their
tilt and roll are ours, their pan differs from our world heading by one
per-file constant, their FOV number means one thing on a flat reframe and
another on a tiny planet, and their tiny planet is not the projection anybody
here assumed it was.

**Their reframe is direction locked.** That is not an aside, it is the shape of
the problem. A locked view is fixed in the WORLD and ours is solved in the
camera BODY's frame, so the body aim of one export is a different set of three
numbers at every instant, different by exactly the flight. Every comparison
across instants below is therefore made on the **world** aim, which is the body
aim with the file's own orientation track (`kjerag_meta::OrientationTrack`, the
same one the app's horizon lock reads) taken back out.

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

## 4a. The registration gates, per instant

`mode=register` prints these for **every instant it is given**, and a run's
answer is the instants that passed. Read per instant on purpose: section 7
already says single frames fail for reasons of content rather than geometry,
and what this asks of them is that they fail loudly.

- **R1 AGREEMENT.** The peak correlation is **≥ 0.30**. Measured on the creek
  export at four instants: 0.9234, 0.9540, 0.9708, 0.9602.
- **R2 PROMINENCE.** The peak stands **≥ 0.05** above the best rival more than
  **12 degrees of rotation away**, that rival refined the same way and checked
  to still be 12 degrees away afterwards. Measured on the creek: **+0.3238,
  +0.4643, +0.3375, +0.6751**. This is the gate that names the old failure:
  the ladder section 8 recorded read 0.7366 at the pose and 0.7380 half a
  degree off it, a prominence of **minus** 0.0014.
- **R3 PEAK.** Over ±2 degrees the ladder falls away from the pose in all three
  axes, and the pose beats ±2 degrees by more than it beats ±0.25 — which is
  G5's own form. Beyond 2 degrees the ladder is printed and not gated, because
  R2 asks about far-away poses over the whole sphere rather than over three
  Euler axes. Measured on the creek: a quarter degree already costs 0.0047 to
  0.0274 and two degrees cost 0.18 to 0.27 more than that.
- **R4 CAPTURE.** The found scale is inside the window that was swept and
  inside the projection's own pole. This is the G7-class gate and it is why the
  window is absolute rather than a factor on their label: **Panini's `d = 0.9`
  cannot draw more than 308 degrees**, and section 8's saturated 328 was a
  number from beyond the edge of its own projection.
- **R7 SCALE PEAK.** The field of view's own ladder falls both ways from the
  answer. Measured on the creek: 4 percent of scale costs 0.19 to 0.34.
- **R0 REPEATS.** At least **three** instants pass R1 to R7.
- **R6 SCALE across instants.** The field of view repeats to **under 2
  percent**. Measured on the creek over four instants: 20.093, 19.734, 20.095,
  19.961 degrees, mean 19.971 — **0.74 percent**.
- **R5 STABILITY across instants.** The **world** tilt agrees to 3 degrees and
  the world roll to 3.5. **The world heading is reported and not gated**, and
  that is a property of the file rather than of the search: tilt and roll are
  referenced to gravity, which the accelerometer measures and the integration
  holds, and heading is referenced to nothing — this camera has no
  magnetometer. Measured on the creek: the world tilt agrees to
  **2.930 degrees** and the world roll to **3.428** across four instants, on a
  told tilt of 3.5 and a told roll of 0, against a heading that spans **25.5
  degrees** over the same four.

## 4b. What Studio's labels mean, in our terms

Measured, not assumed, and each number here is what section 5 below rests on.

| Studio | ours | measured |
| --- | --- | --- |
| tilt | world pitch | told 3.5 → **3.829 ± 1.091** (creek); told −90 → −88.4 (planet) |
| roll | world roll | told 0 → **−0.096 ± 1.262** |
| pan | world heading + one per-file constant | both exports told pan −53.7; both land on the same world heading to **0.14 degrees** at a shared instant |
| FOV, flat reframe | our FULL horizontal field of view | told 20 → **19.971 ± 0.147** degrees, a factor of **0.9985** |
| FOV, tiny planet | neither; see below | told 150 → **259.9 ± 1.0** degrees full over four instants, a factor of **1.73** |
| Distortion, flat reframe | not identifiable at 20 degrees | see below |
| Distortion 0.9, tiny planet | **not Panini at all** — the compression family at `c = 0.600 ± 0.002`, a little wider than **stereographic** (`c = 0.5`) | told d 0.9 scores **0.331**, the found projection **0.837** |

Two of those rows overturn something.

**Their FOV number does not mean one thing.** On the flat reframe it is the
full horizontal field of view, to a factor of 1.00 over four instants. On the
tiny planet it is not: 150 comes back as 259.9 degrees, a factor of 1.73, and
nothing as tidy as a half angle. The "2.2 to 2.4x" that section 8 recorded was
one symptom wearing two causes and the larger of the two was not the label at
all — it was the projection.

**On a tiny planet the scale and the projection are one answer, not two.** A
view pointed at the ground has one strong constraint in it, the radius of the
horizon circle, and a whole ridge of `(c, fov)` pairs satisfies it: `c = 0.5`
with 293.8 degrees and `c = 0.6` with 259.6 both put the horizon at 0.293 of
the half width. What breaks the tie is the ground's own texture inside that
circle, and it breaks it the same way at every instant — `c = 0.600, 0.600,
0.599, 0.595` and `fov = 259.6, 259.3, 259.9, 261.7` across four. **Quote the
pair.** Either number alone is a slice through a ridge.

**Their tiny planet is not Panini, at any `d`.** Its horizon is a **circle**:
measured on the July-14 export at 360 s, the ground disc is 721.5 pixels of
radius across and 717.5 down, round to 0.6 percent. Panini at a nadir aim
cannot draw that — `y = s tan(phi)` sends the horizon to infinity up and down
the picture and leaves it as two straight verticals at `lambda = ±90`
(`panini_cannot_draw_a_round_horizon_at_a_nadir_aim` is the test). The family
that does is the one `mode=parity` already had for unknown projections,
`theta = atan(rho tan(c H))/c`, which is radially symmetric at every `c` and is
exactly stereographic at `c = 0.5`
(`the_compression_family_is_stereographic_at_a_half`). Searched over both
families, the tiny planet lands on `c = 0.600` and the margin is not close:
over the whole scale window, their told Panini `d = 0.9` reaches **0.331** and
`c = 0.6` reaches **0.837**. **A search holding only Panini could not draw
their picture at any scale**, which is what sent its field of view into the top
of its own window.

**Below about 60 degrees of view the projection is not identifiable.** Every
smooth radial map is linear over a small enough picture, so `d` trades against
the field of view with the score not moving: on the creek the best projection
beats the next by 0.007 and 0.000. `mode=register` prints that margin, and the
answer at a narrow view is the aim and the scale, not the projection. It is a
G6-class statement and it is the reason section 5 asks for Distortion 0 rather
than trusting a fitted one.

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
   unknown to a solve that already has one, and section 4b now adds a second
   reason: at the 50 to 70 degrees item 3 asks for, **the projection is not
   identifiable at all**. Measured on the 20 degree creek, the best projection
   beats the next by 0.007 and 0.000. Asking for 0 is asking for the one
   setting where nothing has to be identified.
5. **DIRECTION LOCK OFF.** New, and load bearing. With it on, Studio's view is
   fixed in the world and ours is solved in the camera body's frame, so the
   export has no single aim: it has one per frame, and recovering it needs the
   file's own orientation track in the answer. That path exists and is proved
   — section 4a's R5 is exactly it, and the creek export's world tilt and roll
   repeat to 2.9 and 3.4 degrees across four instants 900 seconds apart, which
   is what confirms the lock was on from the data rather than from the memory
   of a checkbox. But it costs two things that need not be spent. The IMU's
   own error is then inside every aim, and its **heading is not observable at
   all** on this camera — there is no magnetometer, and the integrated yaw
   wanders 25 degrees over the same four instants. With the lock OFF there is
   one aim for the whole export, the IMU is not in the answer, and the frames
   pool without any per-frame compensation between them.
6. **Two aims, not one.** Section 4's G6: at any single view, `lens1 yaw` and
   `lens1 cx` are one number wearing two names. Two aims at different points
   of the seam ring is the cheapest thing that can break that, and it costs
   one more export.
7. **1 to 2 minutes each is right**, and the reason is section 7's second
   finding: single frames fail often, for reasons of content rather than
   geometry, and the analysis has to pool the frames that pass.
8. **Write down what Studio's own boxes said** — pan, tilt, roll, field of
   view, distortion. Not because the pipeline trusts them: section 4b is a
   list of the ways they turned out not to mean what they say. It is because a
   told pan and tilt turn a four dimensional search over SO(3) into a one
   dimensional one, and section 8 records that the four dimensional search is
   information-limited on a narrow view and the one dimensional one is not.

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
- **The SOLVE cannot find a scale it is more than about 15 degrees away from.**
  Measured: starting the field of view 6 deg off recovers it to 0.012 deg, 15
  deg off to 0.050 deg; **30 deg off breaks** (residual 2.63 px against a floor
  of 0.026, on 23 sites instead of 535) and 60 deg off refuses. That is why
  `mode=register` runs first, and why R7 asks its scale to have a peak. It now
  does: 4 percent of scale costs 0.19 to 0.34 of correlation on the creek.
- **The REGISTRATION cannot sweep SO(3) blind on a narrow view.** Measured, and
  it is a property of correlation rather than of this code: two band-passed
  pictures decorrelate over about **one pixel of the working picture**, so a
  sweep `W` pixels across a view `F` degrees wide can only find what it steps
  within about `F/W` of. At `W = 16` on the 20 degree creek that is 1.25
  degrees, and an affordable grid steps 2.5: the export's own answer scores
  0.9205 where it stands and **under 0.65 at the nearest point the grid
  visited**, which is not enough to outrank a second basin that happened to
  land on a grid point. Going finer costs `(1/step)^3`. What removes the
  problem is not a bigger computer, it is **three fewer unknowns**: their view
  is direction locked, so it is fixed in the world, and with their pan and tilt
  written down the only angle left between their frame and ours is the heading
  datum — 360 scores instead of two million. Section 5 item 8 is that
  requirement. On a WIDE view the blind sweep is fine, because there `F/W` and
  the grid step are the same size, and the tiny planet is registered from cold
  with nothing told at all.
- **It cannot promise any given frame will solve.** Of eight runs at the
  recommended geometry across four instants, several refused or diverged
  because of what was in the picture — a hemisphere of sky has nothing to
  correlate. Every one of those failures announced itself: `REFUSED`, or a
  residual 10 to 100 times the floor with the site count collapsed. **No
  failure returned a small residual.** That property, not the success rate, is
  what makes the pipeline safe to point at real data.

## 8. The blocker, and the four things it turned out to be

**It was that the aim-and-scale search had not been built.** The solve is
local: it needs to start within about 15 degrees of the true scale and within a
search radius of the true aim, and on the two existing exports nothing got it
there. What was recorded, on 2026-08-08:

- `mode=parity` fitted the creek export at 0.74 correlation with its field of
  view walking to 48.6 for a slider that said 20, and its pitch to -79 for a
  tilt that said 3.5. Its ladder was not a peak.
- On the tiny planet it reached 0.52 with the field of view **saturated against
  the top of its own search** at 328 for a slider that said 150.
- The registration ladder against the real creek export was **flat within
  +/- 1 degree**: 0.7366 at the pose and 0.7380 half a degree off it.

`mode=register` clears it. On the creek, **four of four instants register**, at
0.9234, 0.9540, 0.9708 and 0.9602, each standing +0.32 to +0.68 above the best
rival more than 12 degrees away, with a field of view repeating to 0.74 percent
and a world tilt and roll repeating to 2.9 and 3.4 degrees. The flat 0.7366
became a peak whose quarter degree costs measurable score. What had to be found
first was that the one symptom had **four** causes, and each was worth a
separate finding.

**One: the objective was the picture and not its structure.** A zero-mean
correlation between two whole pictures is carried by the biggest thing in both,
which outdoors is the sky-to-ground ramp — and that ramp is in the frame at
every aim. That is the whole of the 0.7366-at-the-pose, 0.7380-half-a-degree-off
ladder. Taking a LOCAL mean out and dividing by the LOCAL spread, and scoring
only where their picture has contrast to score, is what turns it into a peak:
without the local spread the peak stood 0.03 above its best rival, with it,
0.32 to 0.68.

**Two: the export was being point sampled.** A 1920 wide frame read 48 pixels
across by nearest neighbour is aliased, and two differently aliased pictures do
not correlate on the content they share. `Export::averaged` is the coarse
reader now.

**Three: the tiny planet is not Panini.** Section 4b. Its horizon is a circle,
Panini cannot draw one at a nadir aim, and no scale in any window was ever
going to fit — which is what the saturated 328 was. The compression family at
`c = 0.50`, stereographic, draws it.

**Four: their reframe is direction locked.** The owner said so and the file
agrees: the body aim is different at every instant and its world tilt and roll
are not. That is also why the blind sweep was the wrong instrument for the
narrow export and the told pan and tilt are the right one — section 7's second
bullet, and section 5's items 5 and 8.

### What the tiny planet clears and what it does not

It clears the **scale and the projection**, and it clears them well: `c` came
back 0.600, 0.600, 0.599, 0.595 and the field of view 259.6, 259.3, 259.9,
261.7 over four instants — a fifth of a percent — against a told Panini `d` of
0.9 that scores 0.331 where the found projection scores 0.837. That is what
section 4b rests on.

It does **not** clear the aim gates: R2 fails at two instants of four and R3 at
two, and it should. R3 asks what a quarter degree costs and R7 asks what four
percent of scale costs, and both thresholds were set for the 50 to 70 degree
geometry section 5 asks the owner for. **A quarter degree of a 260 degree view
is a thousandth of the frame**, and inside the ground disc, where all of that
picture's content is, it is well under a pixel at any width tried (640 and
1280 both). The numbers say exactly this and not something worse: the peak is
0.75 to 0.82, the scale ladder falls from 0.823 to 0.396 over four percent, and
what is flat is the quarter degree and not the answer. A gate that passed there
would be a gate that had been widened to fit, so it is left where it was and
the failure is written down here instead.

### What the controls say

- **The wrong aim scores worse.** The ladder is re-run around every found peak
  and printed beside it, out to 8 degrees in each axis and both ways. R3 gates
  the inner 2. On the creek a quarter degree costs 0.0047 to 0.0274 and two
  degrees cost 0.18 to 0.27 more.
- **A deliberately mislabelled export is found anyway.** The creek run again
  with `fov=999` and `panini=1.35` — both nonsense — over a projection set of
  eight spanning both families. The answer does not move: 33.815, -44.384,
  22.005 deg and fov 19.732 against the labelled run's 33.815, -44.384, 22.000
  and 19.734, and 46.204, 58.284, 14.991, 20.096 against 46.198, 58.284,
  14.981, 20.095. Five thousandths of a degree, on labels that were wrong by a
  factor of fifty. Neither number reaches the search and the run proves it.
- **A window that excludes the answer refuses.** The creek run again with the
  scale window moved off it, high (40 to 90) and low (6 to 13). Neither
  registers at either instant: the high window returns 40.143 — pinned to its
  own floor, which is exactly what R4 fails on — and 52.731, the low window
  7.707 and 6.917, and every one of them is refused by the run as a whole. The
  window is trusted because it is the kind that can say no, not because it was
  widened until it could not.
- **Two-instant controls cannot pass R0 and are not meant to.** R0 asks for
  three instants and the controls are run at two, on purpose: what they are
  asked is whether an INSTANT behaves, and each instant's own verdict is
  printed above the run's.

## 9. The proving run

`scripts/research/pair.py`, `scripts/research/plants.sh`,
`scripts/research/register.sh`, and `--bin seam` modes `register` and `solve`
on branch `research/parity-proof`. Outputs land in gitignored `scratch/`: they
are frames of somebody's real flights and this repo is public.

## 10. THE REAL RUN, 2026-08-09 — the result

Two exports of `VID_20260501_183417_00_002.insv`, made with **Stitching
Optimization OFF, Direction Lock OFF, Chromatic Calibration ON**, both 3840 x
2160 at 30 fps and 176.6 s long, both told **FOV 60, Distortion 0, roll 0**.
The pairing, the registration table and the swap control were run at
`a0366bd`; the plant matrix and the reported answer were run at `6594dce`,
which adds the radial report and leaves every fitted number of the plant at
640 px identical to four decimals. Every output quoted is in gitignored
`scratch/real/`.

Nothing in sections 1 to 9 was changed after the data arrived. What follows is
that criterion applied.

### THE VERDICT: (b) NOT REPRODUCIBLE

The solved residual is **3.92 to 12.39 px rms at the export's own pixel
scale** against a criterion of 1.0, at every geometry, and the solved
calibration beats the shipped factory one by **0.83x to 2.06x** against a
criterion of 3x. It is not close and it is not a near miss dressed as one.

| aim | instant | sites | near seam | azimuth | **solved px** | **factory px** | ratio |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| building | 160 s | 615 | 187 | 185.8 deg | **3.9173** | 8.0523 | 2.06x |
| building | 55 s | 424 | 117 | 271.2 deg | **7.9601** | 11.2575 | 1.41x |
| horizon | 160 s | 548 | 85 | 139.3 deg | **6.7069** | 5.5676 | **0.83x** |
| horizon | 55 s | 268 | 62 | 315.3 deg | **12.3902** | 14.2736 | 1.15x |

Three of criterion 3(a)'s four clauses pass everywhere — 268 to 615 kept sites
against 100, 62 to 187 within 8 degrees of the seam against 40, and 139 to 315
degrees of seam azimuth against 60. The fourth fails by a factor of four to
twelve. Criterion 3(b) fails at all four, and at horizon 160 s the solved arm
is **worse** than the factory one, which is the solver saying it had nothing to
fit: that view keeps 2 sites on lens 1 out of 548 and returns `cx` = 107 px
with a 1 sigma of 23.

**The eye is not the thing that failed.** At 64 px per degree, 3.92 px is 0.061
degrees, and the side-by-side pictures below are the same picture. What failed
is the claim that we can name the reading.

### What the residual is, which is the next diagnosis's input

**Round the seam** (building 160 s, along-seam column, sites within 8 degrees):
order 0 **0.0098 deg**, order 1 **0.0694**, order 2 **0.0426**, order 3
**0.0220**. The roll term is nearly gone and the one- and two-cycle terms are
not.

**Against the field angle it is a radial law, and that is the finding.**

| lens 0 field | 47.5 | 52.5 | 57.5 | 62.5 | 67.5 | 72.5 | 77.5 | 82.5 | 87.5 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| radial, solved | -0.111 | -0.088 | -0.059 | -0.028 | -0.010 | +0.012 | +0.026 | +0.048 | +0.095 |
| radial, factory | -0.110 | -0.087 | -0.057 | -0.024 | -0.002 | +0.022 | +0.040 | +0.066 | +0.088 |
| tangential, solved | -0.030 | -0.015 | +0.009 | +0.026 | +0.029 | +0.026 | +0.010 | -0.017 | -0.037 |

(degrees, horizon 160 s, 532 sites; lens 0 is 546 of that view's 548.)

| lens 1 field | 62.5 | 67.5 | 72.5 | 77.5 | 82.5 | 87.5 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| radial, solved | -0.033 | -0.026 | -0.014 | +0.003 | +0.034 | +0.046 |
| tangential, solved | -0.012 | -0.013 | -0.009 | +0.005 | +0.004 | -0.000 |
| radial, factory | +0.049 | +0.038 | +0.018 | +0.001 | -0.025 | -0.040 |
| tangential, factory | -0.085 | -0.071 | -0.037 | +0.017 | +0.062 | +0.098 |

(degrees, building 160 s, 488 sites.)

Read those two tables together and they say one thing.

- **The tangential column is what the solve removed.** On lens 1 it runs -0.085
  to +0.098 deg with the factory calibration and -0.013 to +0.005 with the
  solved one. A tangential term that changes sign across the field is a
  ROTATION, and lens 1's three angles are exactly the knobs for it. The solve
  worked.
- **The radial column is what it could not touch, and it did not.** On lens 0
  the profile is identical in both arms to the third decimal — as it must be,
  because lens 0 is not in this model at all — and it is a smooth monotone
  function of the field angle running **-0.111 deg at 47.5 to +0.095 at 87.5**,
  crossing zero near 70. On lens 1 it is the same shape and the same size,
  **-0.033 to +0.046**, and the solve moved `cx` by 25 px trying to reach it.
- **A pose difference cannot make a net radial term at any field angle and a
  focal-length difference cannot change its sign inside the field.** A radial
  displacement that is zero at 70 degrees, negative inside it and positive
  outside it is a **different radial polynomial** — and that is exactly the
  question `offset_v6` is: thirteen distortion coefficients where we read five.

So the reading Studio uses is not a pose and not a principal point. **It is the
distortion law itself, on both lenses**, and this solve carries no knob that is
one. That is why the residual will not come down and it is the shape of what to
build next.

### Which file is which, proved rather than guessed

| file | told pan | told tilt | peak | prominence | verdict |
| --- | ---: | ---: | ---: | ---: | --- |
| `Untitled38(8).mp4` | -204.2 | -31.4 | 0.7803 | +0.5048 | **the building area** |
| `Untitled38(9).mp4` | -111.9 | -10.9 | 0.8262 | +0.5857 | **the horizon crossing** |
| `Untitled38(8).mp4` | -111.9 | -10.9 | 0.1613 | +0.0030 | R1, R2, R3 all FAIL |
| `Untitled38(9).mp4` | -204.2 | -31.4 | 0.2338 | +0.0195 | R1, R2 FAIL |

The bottom two rows are the swap, run on the same frame with the same
everything: each file given the other file's label. Neither registers, and the
told sweep walks tilt over only four degrees against a twenty-degree gap
between the two labels, so a wrong pairing could not have quietly succeeded.
`(9)`'s world pitch came back **-10.930** for a told tilt of **-10.9**.

### Pairing (G4)

The exports are the first 176.6 s of the clip, not a middle cut, but the coarse
stage was run anyway and gated: the trim stands **+0.1496** above the best
rival more than one export-length away. The fine lag is **-0.02298 s** on all
eight windows of **both** files, sd **0.00000 s**, peak 0.9865, every window
beating its own +/-1 frame neighbours by at least **1.0302**, fitted drift
**-0.0000 s** across the clip, and injected +/-1 and +/-4 frame shifts
recovered to **0.0010 frames**. Identical to five decimals to the July-14
pair's. **PASS.**

### Registration

`told=` pan/tilt/roll, projection pinned to the told rectilinear, scale swept
12 deg to the projection's own pole. Instants declared before the run as a
regular 35 s grid.

| export | instant | peak | prominence | fov | world pitch | world roll | verdict |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| building | 20 | 0.7327 | +0.5955 | 60.346 | -30.984 | -0.607 | FAIL R7 |
| building | 55 | 0.5857 | +0.4296 | 60.378 | -30.158 | -0.492 | PASS |
| building | 90 | 0.3603 | +0.1964 | 62.285 | -30.573 | -0.634 | PASS |
| building | 125 | 0.6436 | +0.3661 | 60.391 | -30.320 | -0.622 | FAIL R3 |
| building | 160 | 0.7803 | +0.5048 | 60.373 | -30.624 | -0.864 | PASS |
| horizon | 20 | 0.7069 | +0.4856 | 59.817 | -10.988 | +0.474 | PASS |
| horizon | 55 | 0.6069 | +0.3747 | 59.626 | -10.851 | -0.200 | PASS |
| horizon | 90 | 0.6428 | +0.4031 | 59.920 | -10.890 | -0.530 | PASS |
| horizon | 125 | 0.7061 | +0.4905 | 59.807 | -10.961 | -0.412 | PASS |
| horizon | 160 | 0.8262 | +0.5857 | 59.798 | -10.962 | -0.090 | PASS |

R0 3 of 5 and 5 of 5; R6 1.48 and **0.16** percent; R5 world tilt **0.467** and
**0.137** deg, world roll 0.371 and 1.005. **Both REGISTERED.**

Section 4b's mapping holds and gains a sign. Their FOV is our full horizontal
field of view to a factor of **0.9966** on the horizon export (4b said 0.9985)
and 1.0169 on the building one, which carries the 90 s outlier. Their tilt is
our world pitch to **0.030 deg** and **0.948**. Their Distortion 0 is exactly
rectilinear, told and found.

**Their pan runs the OPPOSITE way to our heading.** Both July-14 exports were
told the same pan, so the sign was invisible; these two are told pans 92.3 deg
apart and land 90.9 deg apart the other way. With `heading = -pan + c`, the two
files share one `c` at every instant to **0.72 degrees**: -8.08/-6.85,
+6.86/+8.31, +3.76/+4.52, -8.73/-7.25, -17.94/-16.64.

### Direction Lock, which the pictures answer and the checkbox does not

The checkbox said OFF. The data says their view is **gravity-referenced in tilt
and roll and is not attached to the camera**:

- `lock=body` — the view rigidly attached to the body — **loses outright**:
  0.2141 against 0.5500 on the building export and 0.2040 against 0.6428 on the
  horizon one, with its field of view walking to 12.4 and 162.5 deg for a told
  60 where the world model returns 60.5 and 59.9.
- The world tilt and roll then repeat to **0.137 and 1.005 deg** over five
  instants 140 s apart, which is what horizon levelling looks like.
- The world heading spans **24.8 and 25.0 deg** — but the file's own
  orientation track only turns **3.9 to 9.7 deg** of azimuth over the same
  instants, so the aim held in the camera's own frame spans **21 to 26 deg**.
  A body-fixed heading is refuted, and refuted **immune to the track's drift**,
  because the track's yaw error cancels out of that difference exactly.
- What the heading does instead is the same on BOTH exports: their difference
  is constant to **0.72 deg** at every instant. A wander common to two
  independent registrations of two different pictures is the integration's and
  not the search's — the camera has no magnetometer and section 4a's R5 said
  this would happen.

So the aim is fixed in a levelled frame whose heading our integration cannot
hold to better than 25 degrees over 140 seconds, and matching Studio's aim in
the player needs **Studio's heading integration**, not only its calibration.
The solve is unaffected: it is per instant and starts from that instant's own
registered body aim.

### The plant gates (section 4), at the four real view geometries

| geometry | G1 null rms | G1 worst lens-1 angle | G2 rms | G2 angle error | G2 cx | G2 pitch |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| building 160 s, 640 px | 0.0199 | **0.1031** | 0.0209 | 0.0093 | 8.157 | 0.1907 |
| building 55 s, 640 px | 0.0137 | 0.0351 | 0.0135 | 0.0022 | 7.970 | 0.2010 |
| horizon 160 s, 640 px | 0.0244 | 0.0206 | 0.0239 | 0.0090 | 8.081 | 0.2065 |
| horizon 55 s, 640 px | 0.0234 | 0.0009 | 0.0183 | 0.0040 | 7.860 | 0.1960 |
| building 160 s, **3840 px** | 0.0171 | 0.0001 | 0.0178 | 0.0001 | 7.999 | 0.2000 |
| horizon 160 s, **3840 px** | 0.0285 | 0.0029 | 0.0271 | 0.0050 | 8.013 | 0.2050 |

G2 is read null-referenced, as section 4 says to: planted `cx +8` comes back
**7.860 to 8.157** and planted `pitch +0.2` comes back **0.1907 to 0.2065**,
with every angle inside **0.0093 deg** of a 0.10 gate. G3 REFUSED at both
one-lens views, in this build. G7's second clause holds on every solved arm
(round 1 keeps more sites than round 0) and its first is 96 px of search
against an aim the registration's ladder puts inside a quarter degree, which is
16 px here.

**One gate failed and it is written down rather than widened.** G1 asks every
lens-1 angle back inside **0.08 deg of zero** and at building 160 s at 640 px
`lens1 yaw` returned **0.1031**. At the same geometry at the width the answer
is actually reported at, 3840 px, it returns **0.0001**. Both are above; the
first is a FAIL of section 4 as written, and by section 3(c) that alone would
deny a MATCH at that geometry. It does not change the verdict, which is (b) at
every geometry by a factor of four or more on the residual alone.

### The five knobs, as a family

Building 160 s, the best-conditioned of the four:

```
lens1 roll    +0.1798  +/- 0.0180 deg
lens1 yaw     -0.8132  +/- 0.0356 deg
lens1 pitch   +0.1220  +/- 0.0225 deg
lens1 cx     -25.6830  +/- 0.6763 px
lens1 cy      -0.9097  +/- 0.4564 px

              lens1 roll   lens1 yaw lens1 pitch    lens1 cx    lens1 cy
lens1 roll        1.0000     -0.8854     -0.6311      0.9400     -0.6391
lens1 yaw        -0.8854      1.0000      0.5527     -0.9201      0.4715
lens1 pitch      -0.6311      0.5527      1.0000     -0.4723      0.9440
lens1 cx          0.9400     -0.9201     -0.4723      1.0000     -0.4749
lens1 cy         -0.6391      0.4715      0.9440     -0.4749      1.0000
```

No pair reaches 0.99, so G6 does not force a combination here — but three pairs
sit past 0.92 and the numbers are a family, not five measurements. **They are
not a reading and must not be shipped as one**: the same solve at 55 s on the
same export returns roll +0.046, yaw -0.693, pitch +0.517, `cx` -24.209, and
the two horizon geometries return `cx` -37.4 and +107.0 with 1 sigma of 6 and
23 px. A calibration that does not repeat across four views of one camera is
not a calibration, and the residual above says why: they are chasing a radial
law with rotations.

### What the owner is shown

`scratch/real/deliver/{building-160,horizon-160}/{v3,solved}/` — `theirs.png`,
`ours.png`, `difference-8x.png` and `side-by-side.png` at the export's own 3840
x 2160, with half-scale copies of the pair in
`scratch/real/deliver/preview/`.

At the building aim the two pictures are indistinguishable over the whole frame
except at the pilot's own harness on the left edge, which is a metre away and
is where the two stitches place their seam differently. At the horizon aim the
same is true except for the paramotor's cage strut, which our render draws hard
and Studio's blends away. Neither is a calibration difference; both are what
the near field does to two different seam placements.

## 11. THE RADIAL STAGE, pre-registered 2026-08-09

**Written before the radial model was fitted to anything.** Section 10 ended
with a residual that names its own successor: a smooth monotone per-lens
RADIAL displacement against the field angle, sign-flipping inside the field
(lens 0, −0.111 deg at 47.5 to +0.095 at 87.5, zero near 70; lens 1, −0.033 to
+0.046), and **identical in the solved and the factory arms on lens 0**, which
the five-knob model never touched. Section 10's own reading of that: a pose
cannot make a net radial term and a focal length cannot change its sign inside
the field, so only a different radial polynomial can. This section adds that
knob and says, in advance, what it has to do.

Nothing in sections 1 to 10 is relaxed. Section 3's 1.0 px is carried through
unchanged and everything below is additional.

### 11.1 The model, and why it is the smallest one that can be right

Per lens, a **delta on the radial polynomial** of the shipped Mei/UCM map, in
the v6 form's own radial orders — five:

```text
radial(r) = 1 + k1 r^2 + k2 r^4 + k3 r^6 + k4 r^8 + k5 r^10
```

on the **same normalized plane radius** `offset_v3`'s three are written on.
That is not a choice made here: `docs/research/offset-v6.md` refuted the
angle-polynomial reading before a pixel was read (a radial multiplier of 85 and
788 at the rim against 1.156 and 1.152 for the plane radius), and the five-order
radial head is what `OmniProjection<RadtanDistortPro>` disassembles to. `k4` and
`k5` are new fields, zero on every capture until something fits them.

**The coefficients are fitted from pixels, not read off tokens.** The form is
borrowed; the numbers are the unknown.

Fitted in an **orthogonalized basis**, never as five raw numbers. Gram-Schmidt
of the five monomials `r^2 ... r^10` over a **fixed** window of field angle,
**40 to 95 degrees**, under a measure uniform in field angle, each mode
normalized to unit rms of the radial multiplier over that window. The basis
depends only on the file's own mirror parameter and on that declared window,
and **not** on which sites a run happened to keep — which is what 11.3's
cross-validation needs in order to be a test at all: one geometry's
coefficients have to mean the same thing at another geometry.

Ten knobs, five per lens, on top of section 10's nine. **Nothing else is
added.** The v6 form's tangential, growing-tangential and thin-prism halves are
NOT fitted at this stage. They are considered only under 11.6, and an added
knob with no argument in front of it is fitting the residual rather than the
camera.

### 11.2 THE BAR

The stage answers **REPRODUCIBLE** only if B1 and B2 both pass with B3 and B4
passing in the same run, on the same build and the same frames. Anything else
is **NOT REPRODUCIBLE**, reported with the new residual's structure named.
**There is no middle claim**: a residual that falls from 3.92 px to, say, 1.8 px
is still NOT REPRODUCIBLE, and the fall is reported as a number and not as a
verdict.

- **B1 ABSOLUTE.** With the fitted radial model in place, the residual is
  **≤ 1.0 px rms at the export's own pixel scale (3840 across)** at **all four**
  geometries — building 160 s, building 55 s, horizon 160 s, horizon 55 s — with
  section 3(a)'s other three clauses still met at each (≥ 100 kept sites, ≥ 40
  of them within 8 degrees of the seam, ≥ 60 degrees of seam azimuth).
- **B2 CROSS-VALIDATION, both directions.** The radial coefficients and lens 1's
  five knobs are properties of **one camera** and are therefore fitted **jointly
  over one aim's two geometries** and then **held fixed** while the other aim's
  two geometries are solved with only their own four view numbers free — the
  view genuinely differs per frame; the camera does not. Run both ways round:
  fit on building, predict horizon; fit on horizon, predict building. **Every
  predicted geometry must also reach ≤ 1.0 px rms.** A model that only
  interpolates the sites it was fitted on is a failure of this stage, whatever
  B1 says. A secondary arm that re-frees lens 1's five at the predicted
  geometry is reported beside it as a diagnostic and **is not the gate**.
- **B3 PLANT, in the same run.** A known radial perturbation is injected into
  our own render, which is then used as a fake Studio export, and the whole
  fit is pointed at it from the same start. Planted, declared now:
  lens 0 amplitudes `(+1.0e-3, −6.0e-4, +3.0e-4, 0, 0)` and lens 1
  `(−8.0e-4, +4.0e-4, 0, 0, 0)` in the orthonormal modes of 11.1. Each planted
  amplitude must come back within **15 percent** of what was planted, read
  null-referenced as section 4 says to read a plant, and the residual must
  come back to within **1.5x** of the same geometry's own null. A mode whose
  fitted 1 sigma exceeds a third of the planted amplitude is declared
  **unconstrained by this geometry** and reported as such rather than scored —
  that declaration is made from the printed 1 sigma, not from the error.
- **B4 NULL, in the same run.** The same fit against an unperturbed render of
  our own: residual at section 4's G1 floor (**≤ 0.06 px rms**) and **every**
  fitted radial amplitude **≤ 1.0e-4** in magnitude, which is a third of the
  smallest amplitude B3 plants.
- **B5 CONDITIONING.** The full correlation matrix over every fitted knob is
  printed beside every reported number, and section 4's G6 applies to it
  unchanged: **any pair past 0.99 is reported as a combination and never as two
  numbers.** One near-degeneracy is predicted here in advance so that finding it
  is not a discovery: **the lowest radial mode against the view's own field of
  view**, because a radial multiplier that is nearly constant over the observed
  field is a scale, and a scale is what the field of view already is. If that
  pair is past 0.99 the two are reported as the combination they are.
- **B6 SMALLEST SUFFICIENT MODEL.** A tangential, growing-tangential or
  thin-prism knob is added **only if** the radial fit fails B1 **and** the
  remaining residual still carries named structure that such a term makes — the
  tangential-against-field column running above 0.02 deg peak to peak, or an
  along-seam harmonic of order 1 or 2 above 0.02 deg, both measured against the
  same run's null. Every added knob is argued in writing before it is turned.
- **B7 THE V6 CROSS-CHECK.** The fitted radial delta is compared against the
  file's **own** `offset_v6` string, in the observable rather than in the
  coefficients: the apparent displacement in **degrees of field angle**,
  `theta' − theta` where the two models put the same image radius, tabulated
  over 45 to 90 degrees. A reading of the file's numbers **matches** the fit if
  its profile agrees to **≤ 0.01 deg rms** over that range, which is a third of
  the smallest structure section 10 reported and well under a pixel at the
  export's own scale. The candidate compositions are few and are named now: the
  radial head `k1..k5` is tokens 1 to 5 under **every one** of the sixteen
  readings `kjerag_meta::Reading` enumerates, so only four things can differ —
  v6's radial over v6's own intrinsics, v6's radial over v3's intrinsics, v6's
  intrinsics over v3's radial (the `v6pose` arm), and the inverse direction.
  If one matches, **say which**. If none does, say that plainly: Studio would
  then be computing a refinement of its own, and this protocol does not
  speculate past that sentence.

### 11.3 What the run is, and what it is run on

The same two exports, the same lag `−0.02298 s`, the same four aims section 10
registered and reported at, the same 3840-wide working picture, and the same
build for the plant, the null and the answer. The aims are not re-registered:
they are section 10's own table, and re-fitting them here would be choosing an
aim after seeing the answer.

### 11.4 What is delivered

`theirs.png`, `ours.png`, `difference-8x.png` and `side-by-side.png` at both
aims, at the export's own resolution, drawn with the **fitted** model, in
gitignored `scratch/`. Plus, only if the bar is met, one paragraph saying what
the player must do differently.

## 12. THE RADIAL STAGE — the result

Section 11 was written and committed at `0738ec9`, before the radial model was
fitted to anything. The instrument that fits it is `af7c5b9`; the diagnostics
below it — the structure-against-scatter split and the per-geometry arm — are
`e7f64b4`, and the readout report is the commit above this one. **Every run
quoted was made on `e7f64b4` or later, and the five-knob answer of section 10
reproduces on it to four decimals** (`each-v3-b160` returns 3.9173 px against
section 10's 3.9173, and the other three geometries return 7.9601, 6.7069 and
12.3902 against 7.9601, 6.7069 and 12.3902). Outputs are in gitignored
`scratch/radial/`.

### THE VERDICT: still NOT REPRODUCIBLE, and the residual has a new name

**B1 fails by a factor of 4.5 to 12.7. B2 fails by a factor of 8 to 27.** The
radial knob was built, proved on a plant, pointed at the exports, and it does
not close the gap; and what it fits at one aim makes the *other* aim worse than
fitting nothing at all. The new finding is what stops it, and it is not a
calibration term.

### The gates first, because they are what make the failure mean anything

**B4 NULL — PASS.** All thirty-one numbers free against an unperturbed render
of our own at the four geometries: residual **0.0171, 0.0183, 0.0285, 0.0289 px**
against a floor of 0.06, and the largest fitted radial amplitude anywhere is
**2e-5** against a gate of 1.0e-4.

**B3 PLANT — PASS, and it is the load-bearing gate of this whole section.**
The pre-registered radial perturbation, injected into our own render and
solved for from a start 0.30/0.22/0.10 deg and 0.75 deg of scale off:

| mode | planted | recovered | error |
| --- | ---: | ---: | ---: |
| l0 rad1 | +1.0e-3 | **+1.00e-3** | <0.5% |
| l0 rad2 | −6.0e-4 | **−6.00e-4** | <0.5% |
| l0 rad3 | +3.0e-4 | **+3.00e-4** | <0.5% |
| l1 rad1 | −8.0e-4 | **−8.1e-4** | 1.3% |
| l1 rad2 | +4.0e-4 | **+4.1e-4** | 2.5% |

with `lens1 pitch` back at **0.20023** for a planted 0.2 and `lens1 cx` at
**7.994** for a planted 8, and the residual at **0.0175 to 0.0336 px**, inside
1.2x of the same geometries' own null. The gate asked for 15 percent and got
under 3.

**So the estimator can find a radial law of exactly this family, this shape and
this size, at exactly these four geometries, to within a few percent.** Every
sentence below is said with that behind it: what follows is not an instrument
failing to see something.

**B5 CONDITIONING — bites, and half of it was predicted.** In the four-geometry
fit no pair reaches 0.99 but `l1 rad3` against `l1 rad4` runs **−0.9898** and
`l1 rad2` against `l1 rad4` **+0.9858**: lens 1's five modes are three numbers
at best, and are reported as a family. The degeneracy section 11.2 B5 predicted
in advance — the lowest radial mode against the view's own scale — turned up
against the view's **angles** instead: `l0 rad1` runs **0.887, 0.890, −0.917,
−0.890** against the four arms' view yaws, and only 0.17 against `b160 view
fov`. The prediction was the right shape and the wrong axis.

### B1: the answer, at all four geometries

| aim | instant | sites | near seam | azimuth | **v3, five knobs** | **+ radial, jointly** |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| building | 160 s | 619 | 186 | 182.6 deg | 3.9173 | **4.8995** |
| building | 55 s | 414 | 122 | 291.5 deg | 7.9601 | **7.8673** |
| horizon | 160 s | 480 | 65 | 154.0 deg | 6.7069 | **4.5274** |
| horizon | 55 s | 253 | 59 | 307.7 deg | 12.3902 | **12.7263** |

Ten more knobs, fitted over four geometries at once, move the residual by
**−32 to +25 percent** and leave it four to thirteen times the criterion.
Section 3(a)'s other three clauses pass everywhere, as they did at section 10.

Given **one geometry at a time** — the most generous reading of B1, and the
same freedom section 10's arms had — the ten knobs are not identifiable at all
and the run says so: building 160 s and 55 s **diverge** to 25.2 and 27.0 px
with the kept sites collapsing from 615 to 126, horizon 160 s **REFUSES** at
round 3 when lens 1 runs out of sites, and only horizon 55 s answers, at 8.58
against the factory arm's 12.39. No failure returned a small residual, which is
section 7's own property and is why this is reportable rather than alarming.

### B2: the cross-validation, which is the sharper failure

The camera fitted on one aim's two geometries, held, and the other aim solved
with nothing free but its own four view numbers:

| fitted on | held at | own fit | **predicted** | section 10's factory arm there |
| --- | --- | ---: | ---: | ---: |
| building | horizon 160 s | 3.7666 | **24.4443** | 5.5676 |
| building | horizon 55 s | 11.7608 | **25.2121** | 14.2736 |
| horizon | building 160 s | 26.9886 | **21.9359** | 8.0523 |
| horizon | building 55 s | 14.2427 | **23.5249** | 11.2575 |

**Every prediction is worse than the shipped factory calibration at the same
view.** Re-freeing lens 1's five at the predicted geometry (the diagnostic, not
the gate) changes nothing that matters: 24.9, 25.7, 19.2, 23.3. The reduced
three-mode model is the same story with smaller numbers — 8.1, 12.1, 22.7, 26.8
— and it too is beaten by doing nothing at two of its four.

The parameters say it more plainly than the residuals do. The same camera,
fitted at two aims:

| | `l0 rad1` | `l1 rad1` | `lens1 cx` | `lens1 roll` |
| --- | ---: | ---: | ---: | ---: |
| all four | −0.0127 | −0.0020 | −27.6 | +0.055 |
| building only | **+0.0782** | +0.0157 | −68.5 | −0.141 |
| horizon only | **+0.0011** | −0.0269 | −75.0 | −1.916 |

A calibration is a property of a camera. These are properties of a view.

### What actually stops it: the residual is SCATTER, not shape

The instrument now splits each lens's residual into the part that is a smooth
function of where the site sits — binned in **both** the field angle (5 deg) and
the azimuth round the seam (30 deg), because a rotation has zero mean in the
first alone — and the part that is site-to-site spread inside those cells. On
section 10's own arms, unchanged:

| geometry | lens 0 smooth | lens 0 **scatter** | lens 1 smooth | lens 1 **scatter** |
| --- | ---: | ---: | ---: | ---: |
| building 160 s | 3.96 | **2.87** | 1.86 | **2.84** |
| building 55 s | 4.50 | **7.93** | 1.82 | **6.97** |
| horizon 160 s | 5.48 | **3.35** | — | — |
| horizon 55 s | 4.06 | **3.93** | 9.11 | **22.48** |

**The scatter is never below 2.8 px and reaches 22.5.** It is what the two
pictures disagree by at sites that sit in the same place, and **no calibration
of any shape can reach it** — not a radial polynomial, not a tangential one,
not a thin prism, not a pose. The criterion this protocol fixed at 1.0 px in
section 3 is **three to twenty-two times below the floor these exports set**,
and that was not knowable before section 10 produced sites to measure it on.

That also disposes of B6 without an argument: adding tangential or thin-prism
terms is adding smooth knobs, and the smooth part is already the smaller half
at five of the seven columns above. There is nothing there for them to take.

### Where the scatter comes from, as far as this run can say

The camera is on a paramotor and the readout is not instantaneous. Read off the
file's own IMU, the body turns **0.1011 deg across one 15.88 ms readout at
160 s and 0.2338 deg at 55 s** — **6.4 and 15.0 px** of these exports, edge to
edge. The scatter is 2.8 to 3.9 px at the 160 s instants and 7.0 to 22.5 at the
55 s ones, on **both** files: it tracks the flight and not the aim.

Switching our own readout correction on (`rolling=1`, off by default and off in
every number above) moves the residual by **+2, −5, −10 and −4 percent**
(3.9173 to 3.9854, 7.9601 to 7.5641, 6.7069 to 6.0190, 12.3902 to 11.9331) and
leaves the scatter where it was. So our model of the readout is not their model
of it, and the readout is not the whole of the scatter — but a frame in which
the camera moves 6 to 15 px during its own exposure is not a frame on which a
1.0 px calibration criterion can be decided, and that is the finding.

### B7: the cross-check against the file's own `offset_v6`

**No reading matches, and the interesting number is not that one.** The fitted
delta sits 0.88 deg rms from the closest of the four compositions, which is 88
times the gate — but the fitted delta is itself an artefact (it is ~1 percent of
radial multiplier, ten times the residual it removes, bought by moving every
arm's aim about a degree).

What settles the old question is the **other** column, which is a property of
the file and not of any fit: how far each composition of the file's own numbers
moves a picture at all, against `offset_v3`, in degrees of field angle over 45
to 90:

| composition of the file's own `offset_v6` | lens 0 | lens 1 |
| --- | ---: | ---: |
| v6 radial over v6 intrinsics, forward | **0.0095** | **0.0084** |
| v6 radial over v3 intrinsics, forward | 0.0066 | 0.0446 |
| v6 intrinsics over v3 radial (`v6pose`) | 0.0031 | 0.0376 |
| v6 radial, INVERSE direction | 12.06 | 11.90 |

The radial head `k1..k5` is tokens 1 to 5 under **every one** of the sixteen
readings `kjerag_meta::Reading` enumerates, so those four rows are the whole
candidate space rather than a sample of it.

**The largest displacement any reading of `offset_v6` can produce on lens 0 is
0.0095 deg — 0.61 px of these exports.** Section 10 measured a lens-0 radial
residual running from **−0.111 to +0.095 deg**, which is twelve times that, in
a shape (sign-flipping about 70 degrees) that none of these rows has. The
inverse direction is refuted by twelve degrees of nonsense, which is the same
verdict `docs/research/offset-v6.md` reached from the inside.

So the answer to "how does Studio read the calibration" is, from the outside as
well as the inside: **not out of this string.** Whatever their reframe does
differently, `offset_v6`'s thirteen cannot carry it — they are too small by an
order of magnitude on the lens where section 10's residual is largest. This run
cannot say what they do instead, and does not speculate past that sentence.

### What the owner is shown

`scratch/radial/deliver-radial/{b160,h160}/` — `theirs.png`, `ours.png`,
`difference-8x.png` and `side-by-side.png` at the export's own 3840 x 2160,
drawn with the four-geometry fitted radial model. They are the same pictures
section 10 delivered, because a 4.9 px difference at 64 px per degree is 0.077
degrees and the eye was never what failed.

### What would have to change for this question to be decidable

Not a bigger model. **A frame whose content is not moving during its own
readout**: the same experiment from a camera on a tripod, or from a hover, with
the same two exports made the same way. The instrument, the gates, the plant and
the criterion are all built and all pass; what is missing is data on which 1.0
px is above the floor rather than under it.
