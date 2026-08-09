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
Run by `scripts/research/real.sh` at `a0366bd` and the reported answer at
`c8c8e7f`; every output quoted is in gitignored `scratch/real/`.

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
