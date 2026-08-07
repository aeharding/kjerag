# The seam has two axes, and the campaign only ever measured one

**Status:** diagnosis complete; stages 5 and 6 open against it, sections 9 and
10 written by stage 6. **Date:** 2026-08-01.
**Scope:** why the owner still saw a step on the horizon after issue #103
stages 1 to 4 all merged on good numbers, and what the two instruments built
for the diagnosis measure.

Everything below is measured. Where a number came from one frame of one file
it says so. The footage is the owner's own flights and a four-camera sample
corpus; no filenames, serials or camera keys appear here, and the captures the
tables were read off stay in gitignored `scratch/`.

---

## TL;DR

The seam misregisters on **two** axes and the pass corrects **one**.

- **Epipolar** (across the seam, along the lens-to-lens baseline). This is
  where parallax lives, it is what `band::Cell::disparity` measures, and it is
  what stages 2 and 4 bend.
- **Along the seam** (the circle's own tangent). Parallax cannot reach it: the
  baseline is perpendicular to every direction on the seam circle, so a
  subject's distance displaces it across the seam and never along it, at any
  distance. What is left there is calibration the 5-knob model could not
  reach, plus whatever the pool mixed in.

At the owner's reference view the horizon lands **32.8 view px lower on one
side of the seam** at 1024 across fov 20. The entire band campaign moves that
view by **2.6 px of 33**, because a horizon at that crossing shows **0.6 rows
per degree** of the axis the campaign corrects and **53 rows per degree** of
the one it does not.

The along-seam channel is already measured every frame (`Cell::off_epi`) and
has never been applied. Its search is **3 offsets at 0.30 deg**, and 44% of
measured directions cold / 67% warm sit **on** that limit while the corpus's
real along-seam residual runs **0.17 to 0.67 deg**. The channel is not
measuring that axis; it is clipping it.

---

## 1. The decomposition

At a direction `d` on the seam circle, with the second lens at baseline `b`:

| axis | how it is built | what displaces content along it |
|---|---|---|
| `centre` | `d` itself | nothing; this is the look direction |
| `epi` | `-(b - (b·d)d)`, unit | **distance**. A near subject is displaced towards the front lens, one-signed at every azimuth |
| `perp` | `d x epi`, unit | **calibration only**. Relative roll is a constant here; a principal-point shift is one cycle round the azimuth; focal aspect is two cycles |

`band::Ring` is the Rust twin of the shader's `ring_at` and builds all three.
The property that matters is in the middle row: **`epi` is the only axis a
depth can reach.** So an along-seam disagreement is, by construction, not
parallax — it is the camera, and it is fixed in the camera's frame for the
life of the file. (`--bin seam mode=residual`'s `also=` control is the direct
evidence: two static captures of scenes with nothing in common, minutes apart
with the camera picked up and put down between, read the same along-seam
number.)

Stage 1 fitted five knobs to that axis and left 0.12 to 0.36 deg on the
owner's camera. Stages 2 to 4 then measured and corrected the epipolar axis
per frame, per direction, and never touched what stage 1 left.

## 2. The attribution method: 0.6 rows against 53

A step in a horizon is **one number**, a row difference, and a row difference
cannot say which axis produced it. Attribution is what turns it into a claim,
and it is the method worth keeping from this diagnosis.

At the pixel where the traced horizon actually crosses the seam:

1. Take the view ray there, and the two pixel tangents (one pixel right, one
   pixel down). They are not orthogonal in general, so the resolve is a least
   squares against both, not two dot products.
2. Take that direction's `Ring`, which gives `epi` and `perp` in the body
   frame, and rotate each back into view space.
3. Displace the ray by **one degree** along each axis in turn, resolve the
   displacement into (dx, dy) pixels, and subtract the horizon's own slope:
   `dy - slope * dx` is how many rows *this edge, at this angle* would show.

At the owner's reference view that reads:

| axis | rows shown per degree | the 32.8 px step read as this axis |
|---|---|---|
| epipolar | **0.6** | 55 deg — impossible, the search tops out at 2.6 |
| along the seam | **53** | **0.62 deg** — squarely inside the corpus range |

The horizon at that crossing runs nearly parallel to the epipolar axis, so it
is **edge-on to the axis the campaign corrects** and broadside to the one it
does not. This is not special to that view: it is what a horizon is. The seam
circle is close to vertical in a level camera's body frame, the baseline is
along the lens axes, and the ground's edge runs along the azimuth. A horizon
is the worst possible detector of epipolar error and the best possible
detector of along-seam error, which is exactly why the owner's eye and the
campaign's numbers disagreed for four stages.

`--bin step` prints both rows-per-degree numbers on every run, so no reading
of a step has to be attributed by argument again.

## 3. What the calibration path is worth

Same file, same frame, same view, three ways of reaching the calibration:

| path | step at the reference view |
|---|---|
| factory (no correction) | 49.3 px |
| the pooled per-camera fit that ships | 32.8 px |
| a fit taken from this file alone | 16.3 px |

Two readings of that shape. The pooled fit **is** worth something — a third of
the factory error. And a per-file fit is worth twice as much again and still
leaves 16 px, which is 0.31 deg, which is inside the range section 5 says the
5-knob model cannot get out of.

## 4. The pool mixes knobs no capture endorsed

The owner's camera has a pool of five stored fits holding three distinct
answers. `SeamPool::answer` takes the **median of each knob separately**. The
five knobs trade against each other inside one fit — a roll error and a
principal-point shift produce overlapping signatures on the ring — so
componentwise medians ship a combination that is not any capture's answer:
roll from one fit, yaw from a second, pitch and cy from a third.

Along-seam residual left, in degrees, per flight, per stored fit:

| flight | factory | fit A | fit B | fit C | shipped median |
|---|---|---|---|---|---|
| 1 | 0.816 | 0.504 | 0.611 | **0.324** | 0.468 |
| 2 | 0.778 | 0.406 | 0.681 | **0.176** | 0.413 |
| 3 | 0.817 | **0.248** | 0.589 | 0.188 | 0.297 |
| 4 | 1.044 | **0.314** | 0.575 | 0.378 | 0.357 |
| mean | 0.864 | 0.368 | 0.614 | **0.267** | 0.384 |

**The shipped answer is beaten by a member of its own pool on all four
flights.** Drawing the reference view with fit C instead of the median: 32.8
to 12.8 px cold, 30.2 to 7.3 px warm.

This is a real defect and it is **deferred by owner decision**, not fixed. Two
reasons it is not the thing to fix first: the selection rule is
underdetermined by the data available (one camera, three fits), and a camera
with no pool gets nothing from it. What follows from the standing doctrine
instead is a requirement on the band: **calibration only has to land inside
the band's capture range.** A stage that captures the corpus range makes the
pool's mixing survivable rather than fatal.

**Fixed 2026-08-05**, which reverses the deferral above; the owner tests it
before it merges, as he does every fix. `SeamPool::answer` chooses a member of
the pool, the one the rest of it agrees with most in probe steps, so the answer is
a combination some capture endorsed. The selection rule is still
underdetermined by one camera, which is why it is the pool's own middle and
not a quality ranking: the same sum-of-distances argument the median rested
on, taken over whole fits instead of one knob at a time. Re-measured with the
probe as it stands after issue #130, which reads differently from the table
above, over six of the owner's flights and three places in each: the median
leaves 0.382 deg along the seam on average and the chosen member 0.273, better
on all six flights.

Two limits of that answer, both measured:

- **The choice is metric-dependent.** Which member wins is decided by
  `seam::distance`, which weighs the knobs by the fit's own probe steps, and
  that weighting is not derived from anything the pixels say. On the owner's
  pool the same member wins under raw units, angles alone, principal point
  alone, ten times dearer or cheaper on the centre or on pitch, with pitch or
  yaw dropped entirely, and under a sum of squares instead of a sum of
  distances; it loses to another member when roll or yaw alone is made ten
  times dearer. The residual table is what settles the choice on this camera.
  The metric decides it on the next one, with nothing behind it yet.
- **A pool split evenly has no member to choose**, so it answers with the
  middle of the fits it is split between, which is the knobwise median again
  over those fits. Every pool of two is such a pool, and the app draws with
  the answer from the first capture on, so this is the ordinary state of a
  camera the box has just met rather than a corner case. Over the three pairs
  this pool can make, the middle leaves 0.402, 0.338 and 0.315 deg where the
  members leave 0.382, 0.493 and 0.273: never the worse of the two, and better
  than a coin flip between them on two pairs of the three.

## 5. Generality: the 5-knob model cannot reach it

Along-seam residual left by the app's own best available correction, round the
whole seam circle (`--bin seam mode=residual`):

| camera | calibration path | along-seam left, deg | view px at 1024/fov 20 |
|---|---|---|---|
| owner's X4 Air | pooled, 5 fits | 0.30 - 0.47 | 15 - 24 |
| owner's X4 Air | fitted from the file | 0.17 - 0.32 | 9 - 16 |
| corpus X5 | fitted from the file | 0.29 | 15 |
| corpus X3 | fitted from the file | 0.37 | 19 |
| corpus X4 | fitted from the file | 0.67 | 34 |
| owner's ONE X2 | factory; no fit was possible at all | **2.57** | 132 |
| owner's ONE X2 | fitted from the file (section 11) | 0.26 | 13 |

The last two rows are section 11's and were added to this table on 2026-08-01.
They are the same measurement as the rest of it and they belong beside them,
but the first of them is not a limit of the model: it is a camera the fitting
procedure could not reach.

Three other shooters, three other camera models, and **the best per-file fit
still leaves 0.17 to 0.67 deg**. A better fitting procedure does not close
this; the model is what runs out. It is a rigid-body rotation plus a
principal point, and what is left is whatever the real optics do that five
numbers do not describe.

That range is the requirement a per-frame along-seam channel has to capture,
and it is why the requirement is stated as a range across cameras rather than
as a number from the owner's.

## 6. The search saturates

`PERP_DEG = 0.30`, `PERP_STEPS = 1`, `STEP_DEG = 0.10`. That is three
candidate offsets: -0.30, 0.00, +0.30 degrees. `Cell::off_epi` is written as
`perp * PERP_STEP * STEP` with **no sub-step parabola**, where the epipolar
axis has had one since stage 2.

At the owner's reference view, over the directions with any evidence behind
them:

| state | directions on the 0.30 limit |
|---|---|
| cold (seek + 1 frame) | **44%** |
| warm (2 s of playback) | **67%** |

A reading pinned against the edge of its search window reports the window, not
the content. Against section 5's 0.17-0.67 deg, over half of what this channel
"measures" is the constant 0.30.

Note the asymmetry with the epipolar axis, which **refuses** a pinned reading
(`settle`'s `pinned` test drops it and decays evidence). The off-epipolar
channel has no such refusal because it was never applied, so a clipped value
cost nothing. The moment it is applied, it does.

## 7. What was and was not eliminated

`--bin step` was run against each candidate before the verdict:

| candidate | verdict | the measurement |
|---|---|---|
| cold start / seek | out | seek+1 frame 32.82 px, 2 s of playback 30.18 px; evidence 9 to 42 directions. Worth 8% of the step |
| capture range (epipolar) | out | the epipolar search is 2.6 deg wide and nothing is near it |
| capture range (along-seam) | **decisive** | 3 offsets at 0.30 deg; 44-67% of directions on the limit |
| gating | out | holding the whole band off moves the step 2.6 px |
| calibration | the cause | factory 49.3, pooled 32.8, per-file 16.3, all one frame |
| acceptance regime | the axis, not the zoom | 11.0 px at 1920/90, 27.6 at 1920/45, and **0.59-0.68 deg at every zoom** |

The last row is the one to read twice. Quoting the step in degrees rather than
in pixels makes it the same number at every zoom, which is what says it is a
geometric misregistration and not a magnification artifact — and also what
says a wide-view acceptance number will always look better than the owner's
eye at fov 20.

## 8. The process finding

**Every acceptance number stages 1 to 4 carry is a statistic of the epipolar
axis.** Stage 2's far-field metric, stage 4's doubled-edge recovery, stage 3's
step in codes: all of them read the axis the band searches along, at wide
views where a horizon shows almost none of the other one.

None of those numbers was wrong. They were all measured, all reproduced, all
correct about the thing they measured. The gap is that **the axis a horizon
actually shows was never in an acceptance number**, so four stages of real
improvement on one axis registered as no improvement to the eye that was
looking at the other.

Two rules follow, and stage 5 is held to both:

1. **Acceptance is in picture space, at the zoom the complaint was made at.**
   A degree of disparity is not an artifact; a row of horizon is.
2. **Acceptance covers both axes, and says which is which.** A single-axis
   number cannot distinguish "fixed" from "invisible to this metric".

---

## 9. The two instruments disagreed, and it was three faults (stage 6)

Stage 5 shipped with the band's along-seam channel reading **+0.06 to +0.20
deg** on the azimuths carrying the owner's step where `--bin seam
mode=residual` read **-0.41 to -0.46** on the same directions of the same
file, sd 0.006 over six frames — while on the opposite side of the ring the
two agreed to **0.01 deg**. Opposite signs on one arc, agreement on the other.
Any account of it had to explain both.

Three faults, and each is needed for one half of the symptom.

**(a) A sign that appeared once.** `Ring::perp` was built `centre x epi`,
which is `seam::ring`'s `along` axis pointing the other way. The pass drew
correctly for it — it measures and applies through the same axis, so the two
signs cancel — but every number it printed was the negative of the probe's on
the same direction. Built `epi x centre` now, in the Rust twin, in the shader
and in the phase A study, with a no-GPU test pinning the two instruments to
each other.

**(b) A reset that reached half the ring.** `reset` was a property of a
**frame** and the state it throws away is per **direction**. A frame reads
every `SLICES`-th direction, so a seek reset only the slice it landed on; the
other half kept what it held before the seek and crept toward the new content
at `TAU_FAR`. Measured on the owner's July file: on the reset frame half the
circle read 0.231 deg and the other half read nothing, and after 120 frames
the unreset half was still at **0.56** of what the same content read on the
other. The half is the parity of the cell index and nothing about the footage.
Fixed by sweeping the whole ring on the frame that resets it, and by letting a
direction with no evidence take its first reading whole on whatever frame it
arrives on, which is stage 2's own argument applied where the state lives.

**(c) Two calibrations, read as one.** `--bin band` had no way to be given a
stored fit, so it ran under the file's own fit while `--bin seam` ran under
the owner's pooled one. That difference is **0.04 deg** on the far side of the
ring and **0.32 deg** on the step-carrying arc.

Put together: on the step arc (a) dominates, so the two read the same size
with opposite signs; on the far side the truth is small, and (c) happens to
carry the sign-flipped band value onto the probe's — which is the "agreement".
(b) supplies the +0.06 end of the band's range, which is a direction that had
decayed rather than read.

After (a) and (b), against the probe on the same file, same frames and same
calibration, the band's along-seam readings come back at **0.99 of the
reference on both parities**, where the unreset half read 0.34. The
step-carrying arc reads -0.494 to -0.517 against the probe's -0.462 to -0.493.
`--bin band` takes `seam=` now, so (c) cannot be made again.

## 10. The step instrument extrapolates the terrain (stage 6)

`--bin step` was written on one sentence: a great circle projects to a
straight line in a rectilinear view, so a horizon is straight and a step in it
is the seam's and not the terrain's. **What it traces on real footage is a
treeline or a ridge a few kilometres off, and that is not a great circle.**

The same frame, the owner's reference view, band held off, as the fit window
moves away from the seam:

| `guard` | step |
|---|---|
| 1.2 | 10.4 view px |
| 1.6 | 20.9 |
| 2.0 | 30.5 |
| 2.5 (the campaign's) | 32.8 |
| 3.5 | 37.8 |

The trace itself says why: one side's own slope reads **+5.32 px/deg** over the
two degrees outside the crossover and **+2.03** over `guard=2.5`'s window, and
the fit's rms is 1.51 px in the second against 0.97 in the first. A line
fitted four degrees out is describing the hill.

**What survives this is every difference between two builds**, because the
along-seam correction rotates one hemisphere and moves that side's whole trace
by a constant: **23.2 view px in all three windows**, measured. So the
campaign's before-and-after deltas stand and its absolute numbers carry the
terrain as well as the seam.

A roll sweep with the band held off finds where the picture wants the
correction, and it depends on the window in exactly the same way: the local
step zeroes at roll +0.35 deg past pooled fitted from 1.2 deg out, +0.47 from
1.2 to 5.2, and +0.65 over 2.5 to 6.5. The correlation reads 0.476, which is
the middle one — the patch is 3.7 deg across and averages over about that
reach. **There is no single rotation that registers the two hemispheres at
every distance from the seam**, and 0.028 deg per degree of that spread is
measurable in the probe as well (`off=` -4 to +1 walks the residual from
-0.571 to -0.431).

`--bin step` prints both windows now, `step:` at the wide one and `close:` over
the two degrees just outside this frame's own crossover, with each fit's rms
beside it. The wide window's default moved from 2.5 to **4.2** on 2026-08-06,
because the crossover it has to clear went from 2 degrees to 8 and 2.5 sits
inside that; everything above is `guard=2.5` and reproduces by asking for it.

## 11. The probe assumed the camera knew where its own lenses point (issue #130)

Everything above is measured on cameras whose factory extrinsic is nearly
right. The owner's ONE X2's is not: its two lens axes are recorded **2.835
degrees from opposed**, where his X4 Air's are 0.308, and the seam of every
capture from it reads **2.1 to 2.9 degrees along** and up to 3.4 across.

Under that, the fit refused itself on all three of his captures: 3, 2 and 2
azimuths of 72 against the 10 it needs, so the camera could never build a pool
entry and zero-config playback delivered the factory calibration forever. Two
faults, and each supplies half of it.

**The patch was refused for where its neighbours landed.** `read_ring` sampled
lens 1 as one rectangle grown by the whole search extent and refused the lot if
any corner of it left the picture. At the default that rectangle is 3.84 by
5.85 degrees, most of the overlap band, so 157 of 432 tries were refused as
"not in both pictures" where the X4 Air's were 0. Widening the window made it
strictly worse rather than better: at `along=3.0 across=6.0` **every single try
of 144** was refused for leaving the overlap, and nothing reached the
correlation at all.

**And the window was centred on a calibration that is degrees out.** The search
runs 2.0 degrees either side of where the camera says its lenses point, and the
X2's are 2.1 to 2.9 from there, so 60 of 432 tries peaked against the limit
(the X4 Air: 4 of 768) and the handful that survived reported the limit rather
than the camera. The four readings stage 6 believed - 1.10 to 1.56 along - were
themselves clipped.

The fix is one rule each. The rectangle is still one rectangle and the rays are
the same rays; a summed-area table of the holes in it makes the refusal a
**candidate's** rather than the rectangle's. And a coarse wide pass acquires
where the ring actually sits before the reading pass runs - the median of a
third of the azimuths at a quarter of the sampling rate - and the search is
centred there. Along the seam only, because parallax cannot reach that axis at
any distance (section 1), so a gross offset the whole ring shares there is the
camera and nothing else; and only where the offset is outside the window the
search already covers, so a capture with a good factory extrinsic is read
exactly where it always was.

Measured at the owner's October reference moment, whole ring, six frames:

| | azimuths of 72 | not in both | pinned at the limit | along, deg | across, deg |
|---|---|---|---|---|---|
| factory, as it shipped | 2 | 157 | 60 | 2.570 | 2.830 |
| factory, per candidate + acquired | 35 | **0** | 15 | 2.570 | 2.830 |
| with the fit that then becomes possible | 39 | 0 | **2** | **0.257** | **0.267** |

The three captures now fit 50, 42 and 65 azimuths and **agree with each
other**: roll -2.44, -2.43, -2.49; yaw 1.00, 1.11, 1.23; pitch 2.87, 2.56,
2.85. Three flights on one camera asking for the same five numbers is what
says the answer is the camera's, and no capture from it had ever produced one
before.

Nothing here is a widening, and 10 of the 11 two-lens captures on this box -
eight X4 Air, the corpus X3 and the corpus X5 - come back with **the same fit
to the last digit**. The eleventh is the corpus X4, which turns out to have
been mildly starved too: 33 azimuths become 40, and re-read off the pixels with
each fit in place it leaves 0.591 along and 0.304 across where the fit before
it left 0.601 and 0.318.

---

## How to run the two instruments

Both are on `crates/spike`. Both need real footage and neither commits
anything: PNGs land in gitignored `scratch/`, because these are frames of
somebody's real flights and this repo is public.

### `--bin step` — where a horizon lands either side of the seam

Plays the file into one frame through the **shipped pass**, traces the horizon
column by column as the sub-pixel row of the strongest sky-to-ground step,
fits a straight line each side of the seam with the crossover and a guard left
out, extrapolates both to the seam, and reports the difference — in view
pixels, in degrees, and attributed to both axes by section 2's method.

```sh
# the owner's own view line, verbatim, plus how the band state was reached
cargo run --release -p kjerag-spike --bin step -- <file.insv> \
  time=2.836 yaw=93.99 pitch=4.12 fov=20.00 lock=1 warm=2.0

# the same view cold: a direct seek and one frame, which is what a `reframe`
# render and a launch-by-view-line both are
cargo run --release -p kjerag-spike --bin step -- <file.insv> \
  time=2.836 yaw=93.99 pitch=4.12 fov=20.00 lock=1 warm=0

# with the band held off entirely, which is stage 1's own picture
cargo run --release -p kjerag-spike --bin step -- <file.insv> ... off=1

# against a named calibration path rather than the config's
cargo run --release -p kjerag-spike --bin step -- <file.insv> ... seam=factory
cargo run --release -p kjerag-spike --bin step -- <file.insv> ... seam=pool
cargo run --release -p kjerag-spike --bin step -- <file.insv> ... \
  seam=roll:0.795,yaw:-2.310,pitch:-0.936,cx:-3.28,cy:-11.91

# the binned trace as a table, for reading a terrain rather than a number
cargo run --release -p kjerag-spike --bin step -- <file.insv> ... trace=1
```

**That `seam=` was wrong too, and was corrected on 2026-08-07.** It said
`roll:0.577,yaw:-2.077,pitch:-0.936,cx:-9.53,cy:-11.91`, which is the
knob-by-knob median of the owner's pool and no member of it: precisely the
combination section 4 records `SeamPool::answer` being changed to stop shipping
on 2026-08-05. The string above is the pose the app draws, and `seam=pool` is
the same pose asked for by name rather than copied, which is what a command
meant to stay true should use (docs/research/reference-views.md, the header).
Nothing in this section has been re-read at it.

**That `yaw` is from before 2026-08-06 and points somewhere else now.** The lock
became world-fixed that day, so the frame a `lock=1` yaw is measured in no longer
follows the aircraft's slow heading and its zero is the file's opening heading
instead. `new_yaw = old_yaw + carried(t)`; docs/research/reference-views.md has
the rule and how to measure `carried`. Everything below was read before the
change and none of it has been re-read at the re-derived view.

**`warm` is the argument this instrument exists for.** The band's state is per
direction, paced in media time, thrown away by a seek, and half the circle is
read per frame, so what the pass draws with depends on how many frames of film
ran into this one. `warm=0` and `warm=2.0` are different pictures and both are
real: one is what a launch draws, the other is what watching draws.

What it prints, in order: the crossover's width at this view; the two fitted
slopes and how many columns each kept; **the step**, in px and degrees; the
axis attribution with both rows-per-degree numbers; and what the band's own
state says about the same seam, including how many directions sit on the
off-epipolar search limit.

### `--bin seam mode=residual` — the along-seam residual round the circle

Samples both lenses on the **same angular grid** around directions on the seam
great circle, so the shift that best correlates between them is in degrees of
world angle with no rotation to undo, and splits by construction into along
and across. Decomposes the along-seam reading into harmonics of the azimuth,
each of which names a calibration error (constant = relative roll, one cycle =
principal point, two cycles = focal aspect), and fits a correction through the
shipped map itself.

```sh
# the residual round the circle, its structure, the fit, and the controls
cargo run --release -p kjerag-spike --bin seam -- <file.insv> \
  knobs=roll,yaw,pitch,cx,cy control=1

# the strongest control: a second capture of a different scene on the same ring
cargo run --release -p kjerag-spike --bin seam -- <file.insv> \
  also=<other.insv> knobs=roll,yaw,pitch,cx,cy control=1

# more of the circle, and a finer grid, when the question is a fraction of a degree
cargo run --release -p kjerag-spike --bin seam -- <file.insv> \
  patches=48 step=0.02 from=12 count=8
```

`control=1` injects known errors of the size being measured (half a degree of
roll, twenty pixels of principal point) into lens 1's calibration and reads
them back off the same pixels. **An instrument that cannot see a half degree
it put there itself has not measured the fraction of one it is reporting.**
Run it whenever a residual number is going to be believed.

The `along` column and its `sd` are the along-seam axis. The whole of section
5's table is that column, per camera.

### Reading them together

They answer different halves of one question and a claim about the seam wants
both:

- `seam mode=residual` says **what the misregistration is**, in degrees of
  world angle, round the whole circle, with controls. It is blind to what a
  view does with it.
- `step` says **what the picture does with it**, in rows, at one view, through
  the shipped pass with the band's real state in it. It is blind to every
  azimuth but one.

A fix that moves the first and not the second has not been shown to the eye. A
fix that moves the second and not the first has probably moved one azimuth.
