# The seam has two axes, and the campaign only ever measured one

**Status:** diagnosis complete, stage 5 open against it. **Date:** 2026-08-01.
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
cargo run --release -p kjerag-spike --bin step -- <file.insv> ... \
  seam=roll:0.577,yaw:-2.077,pitch:-0.936,cx:-9.53,cy:-11.91

# the binned trace as a table, for reading a terrain rather than a number
cargo run --release -p kjerag-spike --bin step -- <file.insv> ... trace=1
```

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
