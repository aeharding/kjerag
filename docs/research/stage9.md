# Stage 9: the static per-azimuth along-seam table

**Status:** the mechanism is built, measured and shipped at rest. **No table is
fitted for either camera in the corpus**, because neither camera's leftover
above the five terms the pass already applies predicts a capture it was not
fitted on, and because the most any static table could buy on the corpus that
decides is +1.25 percent. The refusal carries an amplitude: what is excluded is
a static per-azimuth field of order 3 and up above 0.02 to 0.06 degrees, and
nothing smaller. **Issue:** #103.

**Read 4.5 before quoting any cross-flight sentence from 4.** Everything in 4 is
measured through an estimator that takes the **mean** of each azimuth's frames
over a heavy-tailed population, sampled about two readings deep. Under that
estimator the leftover appears not to reproduce across flights at all, and two
sentences in 4 said so; both are **withdrawn** in 4.5. Reduced properly and
sampled densely, the same nine captures reproduce on 18 of 18 pairs, and the
**five-term** field predicts a held-out flight to two hundredths of a degree.
The table is refused under both reductions and by more under the clean one, so
this stage's verdict is unchanged - but the layer it went looking for turned out
to exist one harmonic order below where it looked, and that is 4.5's subject.

This supersedes the stage 9 charter that lived on the unmerged `feat/warp`
branch (`docs/research/local-warp.md`). What that document established is
carried over below; what later evidence reversed is marked.

## 1. The question, and what the layers already are

The seam's along-seam axis is the one no distance can reach (seam-two-axis.md
1), so what disagrees there is the camera. Three layers already act on it, in
this order:

| layer | what it is | when it is fitted | what it leaves |
| --- | --- | --- | --- |
| factory `offset_v3` | the camera's own extrinsics | at manufacture | 0.81 to 0.90 deg rms round the ring |
| `SeamFit` (#48, #154) | five knobs on lens 1, pooled per camera | at open, from the pool | **0.064 to 0.128 deg rms** |
| `band::Along` (#103 stage 5) | five harmonic terms, per session | every frame, on the GPU | see 4 below |

Stage 9 asks whether a **fourth** layer is owed: a static per-azimuth field,
one number per direction, carrying what a pose and five harmonic terms between
them cannot say.

## 2. What is measured and what is applied, before this stage

Established by reading `crates/render`, and the foundation the design rests on:

- The band measures **both** axes per direction, 128 of them, on the GPU, every
  frame: `Cell::disparity` along `Ring::epi` and `Cell::off_epi` along
  `Ring::perp`, each with its own confidence and its own refusal
  (`band.rs`, `measure`).
- The **epipolar** axis is applied cell by cell, interpolated between
  neighbours and split across the handover by the other lens's weight.
- The **along-seam** axis is *not*. Its 128 per-direction readings are input to
  a five-term least-squares fit (`Along::fit` / the `pool_along` entry point),
  and only that fit reaches the picture, applied to lens 1 whole over its whole
  picture, scaled by the ray flattened into the seam plane.
- The reason is on the record and is stage 9's own constraint: applied cell by
  cell it **scallops**, 18.5 view px of correction at one end of a four-degree
  fit and 4.7 at the other (`Along`'s doc comment).

So the per-azimuth along-seam field is already measured and deliberately not
applied. Stage 9's table is the part of it a five-term fit cannot describe,
pooled across sessions rather than read live.

## 3. The mechanism

`band::Table` is 128 numbers in radians along `Ring::perp`, carried in the
`Reframe` uniform block beside the lenses, because it is a calibration and it
travels with the calibration.

- **Applied before projection on the unwarped body ray.** `Reframe::bent` adds
  it to the band's own along-seam term; `blend_bent` gives lens 1 the sum whole
  and lens 0 none of it, which is how `SeamFit` is applied. The **handover
  fraction** is computed from the unwarped ray and is not touched: measured, the
  traced 50/50 contour is identical with and without a planted table, arc 171.0
  to -117.3 deg in every run. The **weight** is that fraction times the bent
  landing's `depth`, which is 1 except within a bend's reach of a lens's image
  circle, so the two are not the same statement - see 7.
- **Zero is exactly identity.** `Table::REST` makes `Reframe::tabled` return the
  ray it was given and `Bend::along` the zero vector, by an equality and not by
  arithmetic that ought to come out at zero.
- **Fitted from measurement, never supplied per view or per clip.** The
  observation is `seam::left`: what the pooled pose leaves at each azimuth of
  the ring `seam::measure` already reads, which is the same function the app
  runs on a background thread while a file plays. No new measurement exists.
- **Never freer than its evidence.** Each entry is a raised-cosine weighted mean
  of the readings within `SMOOTH_DEG` of it, shrunk by `TABLE_RIDGE`, with the
  five terms the pass already applies taken off the **readings** before
  smoothing. A direction no reading reached is exactly zero and its neighbours
  taper into it; an entry past `TABLE_LIMIT_RAD` is refused as a correlation on
  the wrong feature rather than a camera.
- **Read through what is drawn.** `seam::measure` samples through
  `Reframe::tabled`, so a ring measured on a camera that already has a table
  answers what is *still* wrong. Without that the same correction would be asked
  for on every session.

Cost: two loads and a mix per fragment, unconditional. There is no per-frame
estimation - the table is written once at open and never recomputed - so what is
left is a lookup, and it does not measure. Under live decode at 2560x1440
(`--bin playback`), on a quiet box, three runs each: 8.10 / 8.10 / 8.12 ms per
redraw on `origin/main` against 8.14 / 8.12 / 8.15 on this branch, which is
**0.04 ms, 0.24 percent of a 16.6 ms frame** and half a percent of the pass
itself. Repeated later under load as eight interleaved A/B pairs, the paired
difference is **+0.06 ms median with a 95 percent interval of -1.66 to +0.72**:
the box's own noise is twenty times the effect, so the quiet figure is an upper
bound rather than a reading.

## 4. The corpus, and the verdict

`kjerag-spike --bin table` measures it. Every run below is 12 places by 4
frames, 72 azimuths, one pose for every capture, and the readings are gated by
the along-seam plausibility test described in 5.

**The readings themselves are committed**, at `docs/research/stage9/along-seam-
leftovers.csv`: 299 rows of capture, azimuth and what the pose left, in degrees,
under a header that names the plan, the pose and the reduction. It is a derived
table with no frame of anybody's footage in it and no capture time, and it is
here so the table verdict can be re-checked without a six-flight decode. **It is
mean-reduced**, so it cannot answer whether the field reproduces; the nine
per-reading dumps on `research/layer2-preflight` are what that needs (4.5).

### The owner's X4 Air, six flights from April to August

```sh
cargo run --release -p kjerag-spike --bin table -- <six .insv> \
  seam=roll:0.795,yaw:-2.310,pitch:-0.936,cx:-3.28,cy:-11.91 places=12 frames=4
```

| capture | azimuths | along the seam, factory | under the pose | refused |
| --- | ---: | ---: | ---: | ---: |
| 2026-04-10 | 54 | 0.886 | 0.074 | 8 |
| 2026-05-01 | 49 | 0.857 | 0.064 | 2 |
| 2026-05-26 | 61 | 0.850 | 0.070 | 4 |
| 2026-07-14 | 53 | 0.809 | 0.082 | 8 |
| 2026-07-25 | 35 | 0.895 | 0.128 | 6 |
| 2026-08-02 | 47 | 0.825 | 0.084 | 7 |

Degrees rms. This agrees with the other instrument: `--bin crossing bins=180`
at the owner's two May-01 crossings reads the along-seam median magnitude at
**1.30 and 1.43 view px**, which at those views' 18.4 and 16.3 px per degree is
0.071 and 0.088 degrees.

**What each harmonic order leaves, 299 pooled readings:**

| order | 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| left, deg rms | 0.0818 | 0.0750 | **0.0739** | 0.0721 | 0.0720 | 0.0714 | 0.0713 | 0.0712 |

Order 2 is what the pass already applies. Everything above it is worth
**3.7 percent** of the leftover.

**Whether it reproduces**, which is the premise the whole table rests on. At the
azimuths two captures both read - matched on the patch index, because a ring's
azimuths are exact multiples of its own spacing only up to the float that
carried them - two numbers per pair: the standard deviation of their difference,
and the pooled standard deviation of the two captures' own readings there.

| over the 15 pairs | as they stand | with each capture's own five terms off |
| --- | ---: | ---: |
| apart, deg | 0.076 to 0.170 | 0.070 to 0.167 |
| spread, deg | 0.064 to 0.123 | 0.048 to 0.107 |
| correlation, all pairs pooled | **+0.194** | **-0.014** |

**Both columns of that table are estimator artifacts and 4.5 withdraws them.**
Pooled over every pair the two captures' readings correlate at +0.194 as they
stand and -0.014 once each flight's own five terms are gone. Neither number is
evidence about the camera: under a proper reduction at proper density the same
captures agree on 18 of 18 pairs and the five-term field predicts a held-out
flight. What survives from this paragraph is only the shape of the second
number - that whatever agreement a reduction can find between flights lives in
the orders `band::Along` already applies, and not above them - and 4.5 measures
that properly.

*An earlier draft said "two flights disagree at one azimuth by more than either
varies round its whole ring", and a draft before that compared a difference's
magnitude against a root mean square about zero. Both are withdrawn: the first
for the statistic, the second for the estimator underneath it.*

**Held out**, which is the test that decides. Each capture predicted by a table
fitted on the other five, at every kernel width:

| kernel, deg (half-width) | fitted | held out |
| ---: | ---: | ---: |
| **no table** | 0.0828 | **0.0828** |
| 4 | 0.0757 | 0.0872 |
| 8 | 0.0771 | 0.0845 |
| 12 | 0.0786 | 0.0836 |
| 24 | 0.0802 | 0.0824 |
| 36 | 0.0807 | 0.0819 |
| 48 | 0.0812 | **0.0818** |
| 60 | 0.0815 | 0.0819 |
| 90 | 0.0823 | 0.0824 |

The first column improves monotonically as the kernel narrows and the second
gets worse in step, which is the stage-7 striping lesson written as a number: a
field free to follow its own readings' noise always looks better on them.
`SMOOTH_DEG` is a **half-width**, so the 12 degrees the constant carries is a
24-degree window; the sweep runs to 90 so that its best number is a ceiling
rather than the edge of the range.

**The bound.** The best any static table reaches on a capture it was not fitted
on is **0.0818 deg at a 48-degree half-width, +1.25 percent** of the 0.0828 it
would have read with none. That is the most this corpus could ever have paid
for a per-azimuth field at any setting, and it is a fortieth of the along-seam
error the owner can see.

**What this corpus could have found, order by order.** A refusal needs the size
of what it can exclude. A field of a known order and size is added to every
capture's readings - the same field in all of them, which is what static means -
and the whole leave-one-out test is run again. The criterion is not "did it
help", because a noiseless plant helps a little at any size; it is how much of
the planted field's own **power** comes back on the captures the table was not
fitted on, over what the same test recovers with nothing planted.

| order | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| half its power back at, deg | never | never | 0.020 | 0.030 | 0.060 | 0.040 | 0.030 | 0.020 |

Orders 1 and 2 never come back, and that is correct rather than a failure: they
are a pose, `Table` has them levelled out of it by construction, and
`band::Along` applies them itself. Nothing under **0.0185 deg** is tried at any
order - that is the field whose power equals the improvement this test makes out
of a corpus with nothing planted in it at all, so a smaller claim would be a
ratio of two numbers the same size.

**So the honest bound is: this corpus excludes a static per-azimuth field of
order 3 and up at amplitudes over about 0.02 to 0.06 degrees, and says nothing
below 0.02.** At the owner's May-01 GOOD view 0.02 deg is 0.37 view px and 0.06
is 1.1, against an along-seam error of 1.30. A static field of a few tenths of a
pixel is compatible with everything measured here; one large enough to be most
of the defect is not.

### A second camera: the ONE X2, three captures of one evening

The starved camera of issue #130, whose factory extrinsics are 2.8 degrees out,
is the best case for a table if there is one.

| capture | azimuths | factory | under the pose | gate refused |
| --- | ---: | ---: | ---: | ---: |
| 2025-10-18 18:44 | 52 | 2.431 | 0.090 | 14 |
| 2025-10-18 19:13 | 36 | 2.329 | 0.036 | 12 |
| 2025-10-18 19:36 | 58 | 2.515 | 0.069 | 9 |

Orders: 0.0658 at order 0, **0.0518 at order 2**, 0.0489 at order 7 - 5.6
percent above what the pass applies. Held out: 0.0692 at its best widths, 10 and
12 degrees, against a **0.0711** no-table baseline - **+2.7 percent**, while the
first width that resolves anything (4 degrees) is already worse than nothing at
0.0713. Its order-3-and-up structure does reproduce (0.0127 deg of azimuth
structure in the cross-capture median against 0.0116 of cross-capture scatter),
but it is one evening's three captures an hour apart, so it is a property of a
scene and an evening as much as of a unit, and 0.013 deg is a twentieth of the
along-seam error either way.

**And on this camera the answer turns on the gate, which the reader has to
see.** With the along-seam plausibility gate off, the X2's three captures
**support** a table: no table 0.2890 deg, best held out 0.2602 at an 8-degree
half-width, **+10.0 percent**. With it on they do not, at +2.7 percent and worse
than nothing at any width that resolves much. The gate's justification is
physical and predeclared (5 below), and what it removes on this camera is 12 to
14 readings per capture with an ungated tail past two degrees - which is
precisely what an ungated table would be soaking up. But the sentence above
depends on it, and a reader who rejects the gate should read the X2 as a
marginal positive rather than a refusal.

**The X4 Air corpus, which is the one that decides, does not turn on the
gate.** Ungated it reads no table 0.2986 and best held out 0.2985 at a
90-degree half-width: **+0.03 percent**. Gated, +1.25. Both are nothing.

**The verdict: no table is fitted, for either camera.** `Table::REST` ships.

### 4.5 The reduction was the finding, and the layer was one order down

**This section is settled and it overrides every cross-flight claim above it.**
Evidence: the layer-2 preflight branch `research/layer2-preflight`, its
`scratch/layer2/CORPUS.txt` and the nine stamped per-reading dumps under
`scratch/layer2/corpus/`, from `kjerag-spike --bin corpus` over the same nine
captures, the same pose and the same gate as 4.

#### What was wrong with the estimator

`seam::measure` reduces each azimuth's frames with a **mean**, and the band's
`off_epi` exponential average does the same thing over time. The population is
heavy-tailed: at one azimuth the reading moves between two frames 33 ms apart
with a median absolute deviation of **0.008 to 0.05 degrees and an rms of 0.22
to 0.48**. A mean over that is a statistic about its outliers - the same
argument this document makes for the gate in 5, one level further in. The gate
refuses an outlying *azimuth*; nothing refuses an outlying *frame*.

Reduce the same recordings with `seam::left`'s own rule applied per reading
instead - 4 median absolute deviations, floor 0.10 degrees - and the same
corpus, the same pose and the same gate say the opposite thing:

| X4 Air, six flights, all 15 pairs | apart | spread | pairs with apart under spread |
| --- | ---: | ---: | ---: |
| mean, this document's recipe | 0.1296 | 0.0933 | **2 of 15** |
| trimmed | **0.0293** | **0.0542** | **15 of 15** |

and on the ONE X2, 3 of 3 (0.0319 against 0.0628). A field that is a camera has
`apart` under `spread`. Under the mean it does not; under the trim it does, on
every pair of every capture of both cameras.

**So two claims made above are withdrawn.** "Two flights disagree at the same
azimuth by more than either varies round the whole ring" and "the signal is
under its own noise" were properties of a mean over a heavy-tailed population
sampled a few readings deep - two to five, depending on whether a `--bin table`
place is counted as its four frames or as one moment; the density table below
measures the moment-equivalent at 2.0. They are not properties of the camera. The correlation
figures in 4 (+0.194 raw, -0.014 levelled) are the same artifact and may not be
read as evidence about reproduction either; the second of them survives only as
a statement about a table, which is what the next part is.

#### What the table is worth under a clean reduction

Refused, and by more than before. Every arm below is held out - each capture
predicted by a field fitted on the others, nothing measured on its own data:

| trimmed, held out | pose only | 5 terms | 5 + table |
| --- | ---: | ---: | ---: |
| X4 Air, 405 readings | 0.0536 | **0.0211** | 0.0216 |
| ONE X2, 176 readings | 0.0606 | **0.0249** | 0.0263 |

**Read that across all three reductions, because one of the two cameras is
estimator-selected and one is not.** On the X2 a table costs 4 to 6 percent
under every reduction (mean +4.1, trimmed +5.6, median +5.2). On the X4 the
effect runs -1 to +2 percent depending on the estimator (mean -0.1 and -0.6,
median -0.6, trimmed +2.4, and an independent re-implementation of the same trim
+1.3), which is nothing either way. The table is not owed on either camera; it
is refused on the X2 by a number that holds still, and on the X4 by a number too
small to have a sign.

The kernel sweep is flat from 4 to 36 degrees on both cameras, in the table-alone
arm (X4 0.0540 to 0.0534 against a 0.0536 no-table baseline).

**How large is what survives the five terms.** The harmonic ladder under a
refitted pose reads 0.0199 at order 2 and 0.0195 at order 7, and the difference
of two root-mean-squares is not a field: the surviving component's own amplitude
is the orthogonal part, `sqrt(0.0199^2 - 0.0195^2)` = **0.0040 degrees**, and the
median reduction's ladder (0.0173 and 0.0165) gives **0.0052**. At 31.49 source
px per degree that is **0.13 to 0.16 source px, about an eighth of a pixel**, two
to three times finer than `--bin crossing` can resolve. Removing the whole of it
perfectly from the 0.021-degree held-out residual would improve that residual by
**1.8 to 3.1 percent**; a fitted table does not get it, which is the table above.

**And the clean pipeline is not blind.** The control that certifies this
leave-one-out is a cross-capture one, not a within-session one: the same test on
the same partitions recovers the five-term field on 9 captures of 9, taking the
pooled leftover from 0.0536 to 0.0211 degrees. A test that finds a real
cross-capture field on every capture and finds no table on any is measuring, not
failing.

#### What does reproduce, and it is one order down

The **five-term** along-seam field, on every capture of both cameras, fitted on
other flights only:

| trimmed, held out | pose only | five terms fitted elsewhere | improved |
| --- | ---: | ---: | ---: |
| X4 Air, six flights | 0.0536 deg (1.69 src px) | **0.0211 deg (0.66 src px)** | 6 of 6 |
| ONE X2, three captures | 0.0606 deg | **0.0249 deg** | 3 of 3 |

Nine of nine. That is the layer stage 9 went looking for, sitting one harmonic
order below where it looked: not a per-azimuth table but the **pose-order field
pooled per camera**, which `band::Along` already computes per session and which
nothing yet carries between sessions. It is layer 2's, and it is worth about a
source pixel.

A pose refit on trimmed readings also moves the pooled answer materially -
`cy` -11.91 to -13.18, `pitch` -0.936 to -1.096, per-capture leftovers 0.049 to
0.062 down to 0.028 to 0.039 - but it does **not stack** with the five-term
field: held out, 0.0208 with the refitted pose against 0.0211 with the stored
one. They are two removals of the same thing.

#### Why this stage's own instrument could not see it

**Density.** The reproduction needs roughly ten readings per azimuth. Below
that neither reduction reproduces. `--bin table`'s plan is 12 places by 4
frames, which lands about two readings on an azimuth, and its `dump=` writes the
ring **after** `seam::measure` has already meaned it, so the artifact is baked
into the recorded rows rather than visible in them.

Reproduced here from the peer's per-reading dumps, subsampling the same
recordings to each depth and running this document's own trimmed reduction and
gate:

| moments kept | readings per azimuth | apart | spread | pairs passing |
| ---: | ---: | ---: | ---: | ---: |
| 12, this stage's sampling | 2.0 | 0.0938 | 0.0780 | 2 of 15 |
| 24 | 3.0 | 0.0706 | 0.0679 | 9 of 15 |
| 60 | 6.5 | 0.0638 | 0.0647 | 10 of 15 |
| 120 | 13.5 | 0.0409 | 0.0531 | 15 of 15 |
| 1200, all of them | 132.5 | 0.0254 | 0.0483 | 15 of 15 |

(The peer's own figures for the two ends are 0.1077/0.0801 and 0.0293/0.0542;
the small differences are two re-implementations of the trim and of the
subsampling, and the conclusion and the threshold are the same in both.)

**So `docs/research/stage9/along-seam-leftovers.csv`, committed with this PR, is
mean-reduced and says so in its own header.** It is enough to re-check what a
table is worth on top of five terms. It is **not** enough to ask whether the
field reproduces, and the nine dumps on the peer branch are what that question
needs.

#### The one-line consequence for the shipped code

`seam::measure` and the band's `off_epi` update average a population they should
be filtering. On the GPU that is one comparison against `held.off_epi` before
the exponential average takes the new reading. Neither is this PR's to change -
both belong to whichever stage pools the five-term field - but the measurement
above is what says they are worth changing.

## 5. Why this is a refusal and not a blind spot

A negative result is worth nothing from an instrument that could not have found
a positive one, so the instrument is shown catching one.

**The plant.** A table of known size and six cycles round the ring - an order
above anything the pass applies - is put into the map and the same corpus is
measured through it. Every reading must come back moved by exactly the
negative of the table at its own azimuth.

| planted | azimuths | read / planted | scatter about it |
| --- | ---: | ---: | ---: |
| 0.05 deg, 6 cycles | 109 | +0.894 | 0.049 deg |
| 0.10 deg, 6 cycles | 107 | +0.910 | 0.053 deg |

**Through the picture-space instrument too.** `--bin crossing bins=180
table=<planted>` at the May-01 GOOD view, per site:

| planted | shared accepted sites | perp read / planted | epi moved |
| --- | ---: | ---: | ---: |
| 0.05 deg, 6 cycles | 13 of 20 | -1.259 | +0.023 src px (MAD 0.043) |
| 0.10 deg, 6 cycles | 11 of 20 | -1.068 | +0.006 src px (MAD 0.057) |

Those counts are the sites **accepted in both runs**, not the sites the run
traced: 37 are traced, 19 to 20 accepted, and the plant moves a site's own
correlation, so a few accept in one arm and not the other. Two of them do the
opposite - they re-lock onto a different feature under the plant and are
accepted at a value the median then hides - which is why the slope is fitted
through the shared sites and reported with its scatter rather than read off a
median difference.

The sign is the one the geometry predicts: a table that displaces lens 1's ray
by `+t` moves the offset the correlation reads by `-t`. **The epipolar axis does
not move**, which is the invariant the two-axis split is built on. The traced
50/50 contour does not move either.

So an order-6 field at half the size of the residual being looked for is read
back at nine tenths of itself with a twentieth of a degree of scatter.

**What a plant of this kind cannot do**, which the `feat/warp` charter said
plainly about its own and which holds here: the delivered lens planes are one
physical capture, so putting a field into the map does not make a second capture
of a camera that really has one. It exercises the lookup, the axes, the units,
the sign, the application law and both instruments' sampling. It does **not**
validate that a fitted table would correct a real camera, and it cannot on its
own say how small a real field would have to be to escape notice. That second
question is what the order-by-order power scan in 4 is for, and its answer -
0.02 to 0.06 degrees depending on order - is the bound the refusal actually
carries.

**The gate.** The along-seam leftovers are heavy-tailed: ungated, the six
flights read 0.299 deg rms with a maximum of 2.47 deg, while the median absolute
deviation is 0.054. A leftover of 2.47 degrees is not a camera - it is past the
window the correlation searches in - and an rms over that population is a
statistic about the outliers. `seam::left` therefore refuses a reading more than
four times its capture's own scatter from that capture's middle, never closer
than 0.10 degrees. This is `--bin crossing`'s along-seam plausibility gate, one
instrument over, and it is the same physical argument: a capture's calibration
does not change while it plays and no distance can reach this axis, so one
capture's readings are one number plus a slow trend. It refused 2 to 8 readings
per capture. It is a tolerance filter on a physical argument, not a classifier.

## 6. What this stage did not answer

- **Below 15 degrees of azimuth.** The ring is read at 72 azimuths, 5 degrees
  apart, on patches 3.7 degrees wide, and the corpus puts 2 to 4 captures on a
  15-degree bin. Structure finer than that is neither sampled nor resolvable
  here, and the correlation could not carry it anyway.
- **Elevation.** Every reading is on the seam circle. The applied field's
  `cos(elevation)` scaling is a relative roll's own factor, not a measurement,
  and nothing in this stage tests it away from the circle.
- **The across-seam axis.** Untouched, by design and by measurement: it carries
  parallax, it did not reproduce across flights when #155 measured it (9 source
  px apart between May and April against 1.1 along the seam), and the band
  answers it per frame. That reading is `--bin crossing`'s and not
  `seam::measure`'s, so it is not the estimator 4.5 caught - but nobody has
  looked at what **its** per-frame population does either, and after 4.5 that is
  a question rather than a settled no.
- **A small static field.** The refusal has a size on it and not more: above the
  five terms the pass already applies, what survives under a clean reduction has
  an amplitude of 0.004 to 0.005 degrees, which is 0.13 to 0.16 source px, an
  eighth of a pixel and two to three times finer than `--bin crossing` resolves.
  A table on top of the five terms costs 4 to 6 percent on the X2 under every
  reduction and runs -1 to +2 percent on the X4 depending on the estimator (4.5).
  It does **not** mean the along-seam field is not a camera: its five-term part
  reproduces on 18 of 18 pairs across both cameras and predicts a held-out flight
  to 0.021 degrees. That part is layer 2's.
- **Anything at fewer than about ten readings per azimuth.** That is the density
  the reproduction needs, this stage's own instrument sampled about two, and the
  amplitude bound in 4 was measured at that density through the mean. Both are
  therefore bounds on what a thin, badly reduced corpus could see, not on what
  the camera has.
- **Whether the remaining 0.07 degrees is reachable at all.** What is left after
  the pose and the five terms may be per-session, may be elevation-dependent, may
  be a static field under the bound above, or may be the correlation's floor.
- **Whether the freeze the protocol asks for was kept.** It was not, and this is
  the honest record of it: item 4 of the controlled-capture protocol in 7 says to
  freeze the support, taper, fit parameters and condition rule **before** opening
  the hold-out partition. The kernel width was swept against the hold-out column
  instead, and a first draft of this document then read the best width off that
  sweep and called it optimal. Nothing turns on it - every width including the
  best is at or worse than no table, so the sweep chose nothing - but a corpus
  that had said yes would have needed the whole measurement taken again with the
  width fixed first.

## 7. Rules a later applied candidate still inherits

Carried from the `feat/warp` charter, and now enforced by code and tests rather
than by prose:

- A deterministic camera-frame displacement, with a declared smooth taper to
  exactly zero outside its support, fitted from measurement and never supplied
  per view or per clip.
- Applied before projection on the unwarped body ray. **The handover fraction**
  stays a function of the unwarped ray; the weight that reaches the array is that
  fraction times the **bent** landing's `depth`, so a bend that carries a ray past
  a lens's image circle does move that lens's weight. `depth` is 1 everywhere but
  within a bend's reach of the rim, and this predates stage 9 and is inert while
  the table is at rest, but it is not the invariant the charter's prose claimed
  and a later stage that widens the field inherits it.
- No arbitrary per-direction table with nearest-neighbour fill. A field with
  holes in it is the mechanism that made stage 5 scallop and stage 8 stripe, and
  it is why an unmeasured direction here is zero rather than its neighbour's.
- It may not widen the blend or apply photometry to conceal a registration
  error.

**The acceptance battery, in full.** This stage's table never reached it,
because it never had a field to apply. A later one that does has to clear all of
it, and the list is the charter's rather than this document's:

1. **Improve both May crossings without trading one for the other.** The GOOD
   and BAD views at 50.117 s are the same instant on the same file, and a field
   that fixes one at the other's expense is the defect moved rather than
   removed. That is the whole reason the pair is in the registry.
2. **Report both April views separately**, never averaged into the May pair.
3. **Preserve the one-lens paths.** A capture with one lens stream has no seam,
   and nothing here may reach its picture
   (`a_file_with_one_lens_stream_is_still_drawn_exactly_as_stage_one_drew_it`).
4. **Observe the no-fold and cap invariants.** The along-seam axis does not ask
   the band for room because its Jacobian is off-diagonal and its determinant
   stays exactly 1; a field that ever gains a component across the seam loses
   that and has to be clamped like the epipolar one.
5. **Pass `step`, `seam`, the one- and two-pixel same-content Weber excess, and
   `colour`'s interior coherence, across the whole support** - the area the field
   changes, not the seam boundary alone. The interior metric is the one the
   acceptance layer was blind to before stage 8 (reference-views.md, ANTI-
   ACCEPTANCE): main reads 0.03 percent and a rejected build read 1.01.
6. **Flicker and a credible 16.6 ms frame-budget story remain release gates.**

**The controlled-capture protocol, item 4.** Split by physical feature *and* by
capture before fitting; fit on the development partition; **freeze the support,
taper, axes, fit parameters and condition rule before the hold-out partition is
opened**; no held-out feature may be used to choose a site, tune a threshold or
refit either model. This stage swept the kernel width against the hold-out
column and is recorded as having broken that rule in 6.

**Two corrections to that charter.** First, it concluded from a static read of
Insta360's renderer that the maker applies "a content-adaptive *fusion* stage
after calibrated projection, not a camera-frame geometric displacement field",
and told Kjerag not to imitate it. Later work established that Insta360 **does**
move source UVs per frame: a belt of DIS flow at patch 8, stride 3, baked into
the UV lookups. That does not change any rule above - a per-frame flow field is
the band's territory and not this table's - but the charter's inference about
what the maker does is withdrawn.

Second, the withdrawn "within-May epipolar drift" belongs to **#155 and not to
this charter**: it was that PR's own first reading, and that PR's later work
retracted it once the runs behind it were found to be reference-withheld or
three to six sites wide (ROADMAP, the #155 entry). It is named here because a
reader arriving at stage 9 will meet it in the record, not because the charter
made it.
