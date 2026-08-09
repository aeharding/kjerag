# The belt, increment 1: as built, and every number it is holding

**Status:** built, off by default, staged for the owner's playback A/B and
ruled on by nobody. **Date:** 2026-08-09.
**Audience:** the owner, at the checkpoint, and whoever reviews this next.
**Code:** `crates/render/src/belt.rs` (production), `crates/render/src/projection.rs`
(consumption and its Rust twin), `crates/spike/src/bin/belt.rs` (the controls).

**The ruling this is built under**, owner, 2026-08-09:

> You can start the belt. I wouldn't necessarily make the params configurable
> for a lower end device though - lets try to get studio parity first.

So there is **one shape and no ladder of them**. Every number in `belt.rs` is a
constant, nothing is read from the environment except the one switch that
selects the arm, and the floor device is not designed for. `docs/research/belt-rung0.md`
7.2 is a NO-GO for a UHD 620 at any size and says so; this is the
Phoenix-class answer.

**How to read the evidence.** **Measured** means an instrument produced it and
the command is below. **Inference** means it is a reading of measured things.
Nothing here is a projection.

---

## 0. The answer in one page

| the question | the answer |
| --- | --- |
| does the estimator read a known shift true? | **yes**, to **0.0034 degrees** on the worse axis, at both plants and on both arms, with the zero-shift null reading **exactly zero** |
| does an untrusted along-seam segment fail upward? | **yes**, measured: trust 0.000, it takes 0.980 of the coarse answer, and the field there reads **-3.184 of a -3.150 plant** where a snapping gate would read 0 |
| is the off arm the picture main draws? | **yes**, md5 identical at six registry views and over a 180 frame played segment |
| is the shader half guarded? | **yes**, and mutation-proven twice, each the only failing test in the workspace |
| does it hold 30 fps? | **no, not quite**: 29.62 to 29.77 against the control's 29.82 to 29.87. rung-0's bar was 29.8 and this misses it by up to 0.18 fps |
| does it fix the seam? | **nobody has looked yet.** That is the A/B, and it is the next thing |

---

## 1. What runs, per frame

1. **Rectify.** Both lenses' shared overlap into ONE strip, **4096 x 128**,
   off a body-frame map built once when a file opens.
2. **Pyramid.** Two halvings, four taps each, which is the support a
   coarse-to-fine search wants rather than the single tap a mip chain gives it.
3. **Search.** Inverse-compositional Lucas-Kanade over 8x8 patches at stride 3,
   one workgroup per patch, one RDNA wave and two patch pixels a lane. The
   Hessian is built once from the anchor and inverted once. **No variational
   refinement**, because the design has none.
4. **Gate.** One trust per along-seam segment: how many of that segment's
   patches came back under the residual bar.
5. **Densify.** DIS's own residual-weighted mean of the 3x3 patch neighbourhood
   covering each pixel, with an untrusted segment blended 98/2 toward the
   coarse pyramid's answer.
6. **Consume.** The field displaces what each lens is **sampled at**, GPU
   resident, no readback anywhere on the hot path.

**The cadence.** The coarsest rung runs cold every frame; the finest runs
seeded from the previous frame at three iterations. The **whole ladder** runs
on the first frame of a file, after a seek of more than 0.25 s of film in
either direction, and on the frame the arm is switched on.

**Half rate is not offered**, and that is rung-0 3.4's measurement rather than
a preference: the frame that computes gets *more* expensive under a duty cycle
that lets the governor sit at its floor, so a third-rate belt buys average wall
time and nothing at all on the worst frame.

---

## 2. Three deviations from the record, argued

### 2.1 The strip's across axis is the body's elevation, not `Ring::epi`

rung-0's probe rectified onto the ring's epipolar axis, which is the axis
parallax displaces content along, and its doc says rectifying onto anything
else "would put the search's long axis somewhere parallax is not".

**The two axes are 0.6 to 3.5 degrees apart** (`Ring::epi`'s own doc), so 99.8
percent of any parallax displacement still lands on the across axis and the
remainder lands on the along axis, where the search reads it too: the estimator
returns a vector, not a scalar, so the tilt is a rotation of the answer and not
a loss of it.

**What elevation buys is an exact closed-form inverse.** The consuming half
runs per output pixel and has to answer "where on the strip is this ray" with
no iteration. `body = centre(phi) cos a + epi(phi) sin a` cannot be inverted in
closed form, because `epi` is itself a function of the `phi` being solved for;
`body = (cos e cos phi, cos e sin phi, sin e)` inverts in two lines
(`belt_seat`, and `the_strip_parameterization_inverts_exactly`).

The long axis is unaffected: it is the ring either way.

### 2.2 The application is antisymmetric, half to each lens

#171's application form is on record as one-sided - lens 1's whole picture
displaced across the seam (stage9 10.2) - and this splits it instead.

**The argument is about where the correction is given up.** A displacement that
aligns the two lenses inside the handover cannot also be right in the far
field, where there is no second lens to align to. So it is taken back out, and
taking it out over a finite run of picture is a **shear** - the one thing this
project's own principle says a correction must not leave behind
(seam-temporal 1). One-sided puts the whole shear in one lens; antisymmetric
puts half of it in each, which halves the worst gradient and doubles the area
carrying one. **Peak gradient is what an eye finds**, so that is the trade
taken.

**Not measured against one another by an eye.** The constant is
`belt::SPLIT` and it is 0.5; 0.0 is the one-sided arm. This is the largest
open design question in the increment and it belongs in a later A/B, not this
one, which is asking a bigger question first.

### 2.3 The coarse rung runs every frame, which rung-0 did not price

rung-0's seeded arm was the finest level alone. Fail-upward needs a coarse
answer **for this frame** to fail upward to, so the coarsest rung runs cold
every frame. It is 3069 patches against the finest level's 55965, at eight
iterations against three, which is about 15 percent of the finest rung's work.
**Measured cost: the belt reads 5.7 ms a redraw against rung-0's 4.91 for the
seeded-only arm**, and the difference is this.

---

## 3. What is NOT touched, and this is the load-bearing half

**The weights are the geometry's.** `Blend::moved` is a second array beside
`Blend::landings` rather than a field of `Landing`, and the split is the whole
design:

- `landings[i].pixel` is where the geometry puts the sample. The **weight**,
  the **coverage depth** and the **magnification ratio** are all read off it.
- `moved[i]` is where the belt says to sample. **Only the texture coordinate**
  comes off it.

So **flow moves what is sampled; the anchor moves where the crossfade sits**,
and neither reaches into the other. Two consequences that were designed for
rather than discovered:

1. **The crossfade cannot breathe with the field.** `claim` weighs a lens by
   its landing's own coverage depth; a 2.6 degree displacement moves that depth
   by 43 source pixels near the seam, which would have moved the weights by
   tens of percent every time the near field moved. That is the fault the owner
   named directly in 2026-08-05's shimmer complaint, arriving by a new road.
2. **A field with a step in it cannot disengage the magnification upgrade.**
   `texel_ratio` is `dpdx`/`dpdy` of the landing, taken outside the weight gate
   on purpose; if it followed the flow, a discontinuity in the field would spike
   the derivative and switch the cubic kernel off for that quad.

**And with the belt off, `moved[i]` is `landings[i].pixel` assigned rather than
recomputed.** That is what makes the null byte identity rather than
nearly-identity.

---

## 4. The fade, which is the honest cost

`belt::FADE_SHARE` is 0.35: the correction is full from the seam out to 0.65 of
the strip's half-span and is taken to zero by a `smoothstep` over the outer
third, **per lens, on that lens's own side**.

**Why that placement is the whole of what makes it affordable.** Lens 0's
weight is already zero on lens 1's side of the handover and vice versa, so each
lens's fade lives wholly **outside** the handover, where its own picture is the
only picture. Nothing doubled is inside the run of picture the correction is
given up over.

**Why it is a smoothstep and not a taper.** A straight line has a corner where
it starts and another where it stops, and a corner in a displacement is a line
in the picture.

Measured on the owner's own X4 Air: the strip spans **14.12 degrees** across
the seam (12.19 under the pooled fit), the handover reaches 4.00 degrees off
the geometric seam, and the fade runs from **4.59 to 7.06 degrees**. The
correction is full over the whole handover.

**This is not free and the report says so.** A residual of 0.9 degrees split
two ways and given up over 2.47 degrees is a 18 percent local stretch in each
lens's own outer picture. Nobody has looked at it yet.

---

## 5. The controls

`cargo run --release -p kjerag-spike --bin belt -- <file.insv> mode=plant|gate|steady`

### 5.1 The plant table, which is this increment's must-solve

rung-0 4 read the field's along-seam median at **-2.1 to -4.5 strip PIXELS**
across sizes whose along sampling differs by 2.7 times, said that a physical
angle is constant in degrees and this was not, and could not separate an
estimator bias from a real seam. So the estimator is shown to read a **known**
shift true before it is believed about an unknown one.

The plant rectifies **lens 0 a second time** with the across-seam angle
displaced by a known amount, and matches it against the unplanted lens 0. The
answer has to come back at minus that many strip pixels across, and zero along.

Measured, May-01 at 63.5 s, `seam=pool`, 4096x128, 10.50 strip px per degree
across:

| plant, deg | has to read, px | cold ladder | error, deg | seeded | error, deg | along, px |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| **0.00** | 0.000 | **0.000** | **0.0000** | **0.000** | **0.0000** | 0.000 |
| +0.05 | -0.525 | -0.521 | +0.0004 | -0.521 | +0.0003 | -0.000 / +0.001 |
| -0.05 | +0.525 | +0.521 | -0.0004 | +0.520 | -0.0004 | +0.000 / +0.001 |
| +0.30 | -3.150 | -3.186 | -0.0034 | -3.186 | -0.0034 | -0.001 / 0.000 |
| -0.30 | +3.150 | +3.186 | +0.0034 | +3.185 | +0.0034 | +0.001 / -0.000 |

**Worst error on either axis, either arm: 0.0034 degrees**, which is 0.036
strip pixels. The along axis reads 0.000 to 0.001 pixels everywhere it has to
read zero, so the two axes are not crossed. **The zero-shift null reads exactly
zero on both axes with 100 percent of patches kept**, which is the negative
control that says the number above is a reading and not a floor.

**The 0.30 degree plant is recovered COLD, which rung-0's own probe could
not**, and the difference is the ladder: rung-0's plant arm was one level from
a standing start and recovered 1.5 to 1.9 strip pixels of it, which is a
Lucas-Kanade patch's linearization radius. Three levels cover 3.15 pixels
comfortably. That is the measurement that says the ladder is load-bearing.

> **The control caught the bug rung-0 warned about, in this build, before it
> caught anything about the belt.** The first version bound lens 1's PICTURE to
> lens 0's planted COORDINATES, so the control strip was noise: the table read
> +0.137 where it had to read -0.525, the along axis read +0.25 degrees, and
> only 13.8 percent of patches came back under the gate. rung-0 1.1 records the
> identical failure in its own first version. `Map::sources` now carries the
> pairing, and `Belt::rebind` reads it rather than assuming the plane index is
> the lens index, so the map and the binding cannot disagree. **A control that
> cannot fail is not a control.**

### 5.2 The row gate, planted

24 strip columns flattened after rectification - three whole patch strides,
which is wide enough that a finest-level patch fits inside it with nothing in
it and narrow enough that two levels up a patch straddling it keeps most of its
content - on top of a +0.30 degree plant, so the coarse answer there is a
number rather than noise.

| arm | trust | failing upward | coarse, px | delivered field, px |
| --- | ---: | ---: | ---: | ---: |
| untouched | 1.000 | 0.000 | -3.138 | **-3.196** |
| **blanked** | **0.000** | **0.980** | -3.017 | **-3.184** |

The plant is -3.150 strip pixels. **The blanked segment keeps -3.184 of it**;
a gate that snapped to calibration would deliver 0.000 there. The delivered
number is `0.98 * coarse + 0.02 * its own`, to the digit.

### 5.3 Temporal stability

Frame to frame, over 11 consecutive real pairs, on the patches both frames
trusted:

| | May-01 at 63.5 s | July-14 at 36.3 s (the shimmer anchor) |
| --- | ---: | ---: |
| median change | **0.584 px = 0.0556 deg** | **0.725 px = 0.0691 deg** |
| worst per-frame median | 0.612 px = 0.0583 deg | 0.788 px = 0.0751 deg |
| worst per-frame p99 | 3.94 px = 0.375 deg | **8.08 px = 0.769 deg** |

**The control that makes those mean anything**: the same frame pair, hint
thrown away, run twice moves the field by **0.000000 strip pixels** on both
files. The estimator is deterministic, so everything in the table is the world
going past the belt rather than the estimator changing its mind.

**It is not zero and this document does not claim it is.** The median is 0.06
to 0.07 degrees a frame, which is under a source pixel of relative motion. The
p99 is the finding: **8.08 strip pixels at the shimmer anchor is four times the
1.5 to 1.9 pixel radius a seeded search can recover**, which is exactly the
open risk rung-0 4 named with a number (21.8 deg/s of world sweep is 8.3 strip
pixels a frame at this sampling). On about one patch in a hundred the hint is a
hint about different content. What catches those is the residual gate, and what
they get instead is the coarse rung's answer. **Whether that reads as calm or
as wobble is the owner's question and not this document's.**

---

## 6. The null

Two halves, both measured against `main` at `8887745`:

- **Six registry views**, `--bin reframe` at 1024 px, `seam=pool`: `down1`,
  `down2`, `down3`, `shimmer`, `good`, `bad`. **md5 identical, six of six.**
- **A played segment**, `--bin null` over 180 frames of May-01 from 63.5 to
  69.5 s: summary `42513503b0e54e20b184135814817991`, **identical**. Playing
  rather than seeking is the point: the band warms over seconds and the line is
  held across frames.

**And it is not vacuous.** The same played segment with `KJERAG_BELT=on` reads
`6cfb0356539bd0812e3ad77a6e76c393`.

---

## 7. The twin guard, grown and mutation-proven

`crates/render/src/twin.rs` compiles the shipped `projection::wgsl()` with a
probe entry after it and compares against the Rust mirror on a real GPU. The
belt's consuming path is inside `blend`, so it is inside that boundary - but
only with the right fixture and the right question.

**The question is split in two, because the shader reads a texture the mirror
has not got.** `blended(ray, flow, on)` is asked about a flow both halves are
simply **handed**; `belt_look(ray)` - the field read itself - is asked
separately, against a texture planted at values `Rg16Float` holds **exactly**
(multiples of 1/64, no larger than 16, with `the_planted_field_survives_the_format`
as the control that says so). So what is compared is arithmetic and never a
rounding.

**The fixture asserts the block says the belt is on**, which is this file's own
lesson applied for the third time: it has already recorded a fixture that did
not roll taking the readout out of the comparison, and a fixture that named one
model taking the other out.

Clean, on RADV Phoenix over 5930 rays: **3816 rays on the strip, 3824 samples
moved by up to 11.56 px**, worst moved landing **5.4e-3 px** against a 1e-2
bar, worst field read **1.1e-4 strip px** against a 2e-3 bar.

| mutation, WGSL only | reads | of the bar |
| --- | ---: | ---: |
| lens 0's share of the flow the wrong way round (`belt_gain`) | 23.15 px | **2315x** |
| a bilinear that fetches right and mixes wrong (`belt_flow`) | 0.052 strip px | **26x** |

Under each, the twin is **the only failing test in the workspace** (236 pass,
1 fails). Each was reverted.

---

## 8. The clock, disclosed rather than dressed

`--bin playback`, 20 s of real decode, three rotated reps, 2560x1440, at the
`down1` framing, sound through the null sink:

| arm | fps presented | dropped / 20 s | starved | underruns | ms per redraw |
| --- | ---: | ---: | ---: | ---: | ---: |
| off (control) | 29.82, 29.87, 29.82 | 3, 2, 3 | 0 | 0 | 8.03, 8.63, 8.01 |
| **flow** | **29.72, 29.62, 29.72** | **5, 7, 5** | **0** | **0** | 14.40, 13.87, 14.25 |

**rung-0's bar was 29.8 fps and this misses it**, by 0.08 to 0.18 fps against
the control, with two to four more dropped frames in twenty seconds. Nothing
starved and no audio underran on any run.

**The belt costs 5.7 ms a redraw and its consuming half is free.** Measured by
computing the field and not reading it: **13.75 ms** with the field computed
and the block saying off, against **13.82 ms** with it read. That is inside the
noise, and it reproduces rung-0's own finding that the consuming lookup is
0.008 to 0.041 ms over the handover corridor.

**Where the 5.7 goes against rung-0's 4.91**: the coarse rung every frame (2.3,
inference from patch counts and rung-0's per-iteration figures) and this build
having eleven pass blocks in one uniform buffer rather than one rewritten
between passes. That second one was worth **0.3 fps and five dropped frames**
when it was wrong: a `write_buffer` lands at the next submit, so a block shared
between passes forces a submit between them, and nine queue flushes a frame
read 29.42 to 29.47 fps where one reads 29.62 to 29.77.

**The map costs 50 to 70 ms once**, when the arm is first selected, on one CPU
thread. It is not paid at all while the arm is off.

---

## 8.5 What the picture looks like, and the one that got WORSE

Decoded stills, both arms, at four views, in gitignored `scratch/belt-stills/`.
Looked at by eye. **All four pairs differ; none is byte identical.**

| view | what changed |
| --- | --- |
| **gear** | **the headline, and it works.** The red glove and the strap below it go from a thin washed-out sliver half dissolved into the harness behind them to **solid, opaque and saturated**, with folds; a pale item at the top of the strap appears that the off arm had smeared away; the reflective stripe on his sleeve goes from a translucent ghost to a crisp band. This is what `~/Videos/studio_onoff/optical_flow_near/on.jpg` looks like against its own off. |
| **bad** | **the far-field doubling closes, visibly.** The far ridgeline goes from a smeared doubled grey wash to one crisp ridge **with a water tower silhouette resolved on top of it that does not exist in the off arm at all**. That is the roughly 20 view px epipolar defect the registry documents. The risers at top right are identical on both arms, because they sit outside the strip. |
| **good** | **no visible difference, which is the right answer for a control.** The belt does reposition band content by 3 to 7 px with no sharpening to show for it, which is invisible in a still and is worth watching in motion. |
| **down1** | **MIXED, and the damage is worse than the gain.** |

### The down1 regression, which is the largest open problem in this increment

**The tree canopy is put through a liquify filter.** At `down1`'s fov 20 the
belt draws wormy candle-wax ripples and dark curved streaks across fine
repetitive foliage that are simply not in the off arm, and the parked cars in
the same band go from distinct white blobs to diagonal smeared streaks. In the
same picture the store's roof edge against the trees goes from a mushy
transition to a defined straight line, and the roof units sharpen: the belt is
displacing that roof by 23 px (0.45 degrees) at correlation 0.92, which is a
confident and correct alignment move.

**So the belt is right about strong high-contrast structure and destructive on
fine self-similar texture**, in one frame, in one band.

**The mechanism, and it is an inference.** The gate is per along-seam SEGMENT -
one trust for a whole column of the strip - so a segment carrying mostly good
matches passes as trusted while individual patches inside it, on foliage where
the match is ambiguous, wander within the capture clamp and are densified into
the field anyway. The 3x3 residual-weighted mean smooths them; it does not
reject them. **A per-patch gate, or a smoothness term on the field, is what
this is asking for, and neither is in this increment.**

**It is the fault the owner refuses by name.** seam-temporal 1 is explicit:
corrections displace, residuals ghost, and a correction that cannot be right
everywhere should leave a double image and never a bent one. The canopy is a
bent one. It is disclosed at the top of the PR, it is in the owner-facing
briefing, and `down1` is a trial in the A/B session, so it is a picture he is
shown rather than a paragraph he is told about.

> **A metric that agreed with the good news and not the bad.** Gradient energy
> inside the band reads +1.8 / +0.6 / +1.6 / **+3.1** percent at bad / good /
> gear / down1, so it scores the view that visibly got WORSE highest of the
> four, because wormy warping manufactures gradients. It is recorded here as a
> metric that does not work for this question rather than quoted as evidence.

## 9. What this does not claim

- **It does not claim the belt fixes the seam.** The stills in 8.5 are four
  frames looked at by one pair of eyes that are not the owner's, and the whole
  epic's record is that his eye has overruled the instruments before. That is
  the A/B and it is the next thing.
- **It does not claim the picture is uniformly better.** 8.5 has one view where
  it is clearly worse, and the mechanism named there is not fixed.
- **It does not claim antisymmetric beats one-sided.** The argument in 2.2 is
  about peak gradient and it is not a measurement of a picture.
- **It does not hold 30 fps to rung-0's own bar** (section 8).
- **It does not claim the seed is safe across a hard turn.** Section 5.3
  measures the p99 at four times what a seeded search can recover and says what
  catches it, not that nothing is lost.
- **It has not been near a floor device**, and by rung-0 7.2 it never will be
  at this shape.
- **It says nothing about the ONE X2 or the DJI Osmo 360.** The strip's span is
  computed per camera and the twin runs both models, but no `.OSV` and no X2
  has been played with the arm on.
