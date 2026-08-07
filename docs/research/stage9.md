# Stage 9: the static per-azimuth along-seam table

**Status:** the per-azimuth table is refused and the vehicle it was built on
carries a different field. **No per-azimuth table is fitted for either camera in
the corpus**, because neither camera's leftover above the five terms the pass
already applies predicts a capture it was not fitted on, and because the most
any static table could buy on the corpus that decides is +1.25 percent. The
refusal carries an amplitude: what is excluded is a static per-azimuth field of
order 3 and up above 0.02 to 0.06 degrees, and nothing smaller. **What is
shipped in `band::Table` is the five-term field of 4.5, pooled per camera and
learned by watching; section 8 is that layer and its numbers.** **Issue:**
#103.

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
perfectly would improve the held-out residual by **1.8 to 4 percent, depending on which arm's residual it is measured against** - each reduction's
amplitude against its own arm gives 1.8 percent trimmed and 4.1 median. A fitted
table does not get it, which is the table above.

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
the exponential average takes the new reading. Neither was that PR's to change;
both are section 8's, and both are done.

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

## 8. Layer 2: the field that reproduces, measured through the shipped path

**Read section 9 before reading any applied claim in this one.** Everything
below is measured on the **unbent** projection - `seam::measure`'s ring and
`--bin crossing`, both of which draw with the per-frame band held off. In that
domain the field is real and reproduces on nine captures of nine. In the
**delivered** picture it buys nothing, because the band has already taken the
same leftover out, and section 9 is that measurement and what it cost. The field
is measured, guarded and stored; nothing applies it.

### 8.1 The estimator, where there is a median to take

`seam::tolerated` is one rule in one function: the middle of a set of readings
is a median, the spread is a median absolute deviation, and a reading further
than `GATE_MADS` spreads from the middle - never nearer than `GATE_FLOOR_DEG`,
which is 0.10 degrees - is a correlation on the wrong feature rather than a
camera.

- **Across a ring's azimuths**, which is `seam::left` and was already there.
- **Across one azimuth's frames**, which is `seam::reduced` and is new.
  `seam::measure` meaned them. A direction whose frames all agree reduces to
  exactly the mean this replaced, by the same additions in the same order.

**Both are load-bearing and neither replaces the other**, which is the peer
branch's own finding and is why the ring gate is untouched. On the six flights
at `--bin table`'s own plan the trim alone takes the pooled leftover under the
stored pose from 0.0828 to **0.0653** degrees, and the corpus's cross-capture
agreement from 2 pairs of 15 to 15 of 15 (4.5).

**The GPU half of 4.5's request is not here.** One comparison against
`held.off_epi` before the exponential average re-introduces snapping, which is
`seam.rs`'s own forbidden artifact: measured at the shimmer view with no field
on either side, the band's frame-to-frame state went 0.000760 to 0.015042 deg
rms, its worst single step 0.0032 to 0.1028, and the applied picture stepped by
over a view pixel on 12 of 87 frame pairs where the shipped pass steps on none.
It belongs to a stage that can instrument it.

**What the trim refuses and what it keeps.** The along-seam axis decides and a
refused moment takes its across-seam reading with it, because a frame that did
not correlate on the content it was pointed at did not correlate on it for
either axis. **The converse is not true and is the scope of this**: a moment
whose along-seam reading is ordinary and whose across-seam reading is wild is
kept whole, because that axis carries parallax and the physical argument this
rule is built on does not hold there.

**What the trim buys in the picture is not measured.** Everything above is a
statement about readings. What it changes in the app is the fit those readings
produce, and section 9.5 measures that change and does not say which fit is
better in the delivered picture. That is the live question this PR leaves open.

### 8.2 What the pool stores and what nothing carries

| | what it is | where it lives | fitted from |
| --- | --- | --- | --- |
| `SeamSample::along_deg` | five terms, degrees, **above the factory calibration** | the app's config, beside the five knobs | `seam::along_kept`, off the ring the fit already reads |
| `band::Table` | 128 numbers, radians, along `Ring::perp` | the `Reframe` uniform | **nothing. `Table::REST` ships** (section 9) |

**The stored number is pose-free.** A leftover is a quantity relative to
whichever pose was taken off it, so two captures' leftovers are the same thing
only under one pose - and the pose a camera is drawn with moves as its pool
grows. `Reading::along` does not move: `seam::measure` reads every ring through
the calibration the camera wrote, on every capture, for the life of the camera.
The pose is still what **gates** the reading, because the plausibility argument
is about what is left. `seam::along_table` composes the two on demand and is
now called only by the guard below and by the instruments.

Two details that were measured rather than argued:

- **The pose is a five-term field to one part in 1904** - 0.00043 degrees of a
  0.8212 degree signature
  (`a_pose_is_a_five_term_field_to_a_part_in_two_thousand`).
- **No ridge on the field fit**, because one azimuth's worth of shrinking on a
  0.85 degree field is 0.012 degrees, which is most of what a field is worth.
  What refuses a starved ring instead is a guard at harvest: `seam::along_kept`
  refuses a sample whose own five terms compose to more than `FIELD_LIMIT`
  times the leftover they were fitted to. Measured as `composed / leftover`:

  | capture | covered, deg | app plan 3x2 | 12x4 | 24x20 |
  | --- | ---: | ---: | ---: | ---: |
  | X4 2026-04-10 | 285 to 340 | 0.80 | 0.78 | 0.93 |
  | X4 2026-05-01 | 280 to 340 | 0.83 | 0.85 | 0.91 |
  | X4 2026-05-26 | 320 to 350 | 0.64 | 0.93 | 0.92 |
  | X4 2026-07-14 | 275 to 345 | 1.02 | 0.76 | 0.81 |
  | **X4 2026-07-25** | **105 to 240** | **refused** | **1.33** | **1.10** |
  | X4 2026-08-02 | 275 to 330 | 1.00 | 0.75 | 0.68 |
  | X2, three captures | 280 to 310 | 0.61 to 0.83 | 0.79 to 0.92 | - |

  `FIELD_LIMIT` is 1.2, in the gap between 1.02 and 1.33. At the deepest plan
  the July-25 flight reads 1.10 and passes, which is the guard's own limit: a
  ring deep enough stops looking starved by this test before it stops having a
  hole in it.

### 8.3 The ladder, on the unbent projection, every arm held out

`kjerag-spike --bin table` pools through `seam::along_kept`, the same guard the
app harvests through. Every column is held out.

| held out, deg rms along the seam | azimuths | pose only | field | mean control | field + table |
| --- | ---: | ---: | ---: | ---: | ---: |
| X4 Air, six flights, 24x20 | 372 | 0.0644 | **0.0375** | 0.0391 | 0.0387 |
| ONE X2, three captures, 24x20 | 190 | 0.0414 | **0.0140** | 0.0136 | 0.0137 |

**Nine captures of nine improved.** At 12 by 4: 0.0653 -> 0.0432 and
0.0675 -> 0.0477, 9 of 9 again. At the app's own `Plan::default`, 3 places by 2
frames: X4 **0.0844 -> 0.0707, 4 of 6** (Aug-02 worse by 10.2 percent, May-01 by
2.6) and X2 0.0325 -> 0.0297, 2 of 3.

**All of it is the unbent projection.** Section 9 is what happens when the same
field is put into the picture.

Over the six corpus-and-plan arms the mean control beats the app's middle on the
pooled number in four:

| pooled, deg rms | middle | mean |
| --- | ---: | ---: |
| X4, 24x20 | **0.0375** | 0.0391 |
| X4, 12x4 | 0.0432 | **0.0430** |
| X4, app plan 3x2 | **0.0707** | 0.0715 |
| X2, 24x20 | 0.0140 | **0.0136** |
| X2, 12x4 | 0.0477 | **0.0465** |
| X2, app plan 3x2 | 0.0297 | **0.0275** |

Per capture the middle wins 5 of 9 at either plan. A per-azimuth table on top of
the field costs 2 to 3 percent on the X4 Air and **gains 2 to 5 on the ONE X2**.

### 8.4 At the registry, still on the unbent projection

`--bin crossing bins=180` at the two re-derived May-01 crossings under
`seam=roll:0.795,yaw:-2.310,pitch:-0.936,cx:-3.28,cy:-11.91`, field pooled
through the shipped guard off flights that are not May-01. **The band is held
off in this instrument** (`Held::default`), and section 9 is why that sentence
turned out to be the whole story:

| view | along-seam median magnitude, view px | epipolar median, view px |
| --- | ---: | ---: |
| GOOD, no field | 1.29 | -6.00 |
| GOOD, field off five other flights | **0.12** | -6.11 |
| BAD, no field | 1.47 | -10.17 |
| BAD, field off five other flights | **0.93** | -10.07 |

The epipolar axis is untouched range to range and not run to run: it spans 0.13
view px at GOOD and 0.15 at BAD against along-seam moves of 1.17 and 0.54, but
three of four arms move it by more than the smaller of the two runs' own dither.

## 9. The delivered picture, and why the field is not applied

**This section overturns every applied claim in 8 and it is the binding one.**
Section 8's instruments all draw the **unbent** projection: `seam::measure`
reads a ring through `Reframe` with no band, and `--bin crossing` builds its map
with `Held::default()`. The app does not. In the picture the pilot sees, the
per-frame band measures the same seam every frame and applies its own five-term
`Along` fit, and **that fit had already taken the along-seam leftover out**.

### 9.1 What the delivered comparison read

Both builds, the app itself, the same clip and the same view, photographed:

| view | main, along-seam at the probes | branch with the field applied |
| --- | --- | --- |
| GOOD | +0.44, +1.83, +0.43, +0.42 view px | +0.43, +1.82, +0.43, -0.29 |
| BAD | -2.59, +0.05, **-0.11**, +0.05 | -3.12, +0.05, **-2.06**, +0.05 |
| shimmer | -39.01, -31.61, -38.93, -31.61 | -39.55, -32.70, -39.78, -32.47 |

At GOOD the delivered along-seam axis is **already at or under 0.6 view px on
`main`** and the field arm matches it within 0.2, against an instrument shown
capable at 1 px. At BAD `main` reads -0.11 where the unbent projection reads
1.47 - the band had zeroed it - and the field arm reads **-2.06, about two view
pixels the wrong way**. At the shimmer view the field arm is slightly worse on
every probe.

### 9.2 Why the read-through did not prevent it

The compute pass was made to sample lens 1 through the table, so that the band
would fit the residual and apply only what the table still left. That is
necessary and it is **not sufficient**, and the reason is arithmetic:

> With a table `T` applied and the band measuring through it, the delivered
> correction is `T + fit(L - T)` against `fit(L)` with none. By linearity the two
> differ by exactly **`T - fit(T)`**.

`fit` is `Along::fit`: five terms, weighted by each direction's own confidence,
shrunk by a ridge. It reproduces `T` only where it has evidence.

**Two fields, and the columns say which is which.** The first is the real pooled
field this branch composed, as `--bin table field=` wrote it, 0.2735 deg rms
composed - that is the one the delivered readings in 9.1 were taken through, and
it is read out of a scratch file. The second is the plain five-term field of
0.2163 deg rms that
`a_partial_ring_cannot_fit_away_a_table_over_the_whole_of_it` plants, which
needs no footage and is what `cargo test` keeps honest:

| ring directions with evidence | real field, rms | worst | test's field, rms | worst |
| ---: | ---: | ---: | ---: | ---: |
| 128 of 128 | 0.0007 deg | 0.0011 | 0.0020 | 0.0037 |
| 96 | - | - | 0.0127 | 0.0279 |
| 64 | 0.0080 | 0.0175 | 0.0677 | 0.1403 |
| 48 | 0.0247 | 0.0514 | 0.0676 | 0.1440 |
| **27**, what `--bin step` reports on real footage | **0.0333** | **0.0696** | **0.0856** | **0.1710** |
| 16 | 0.0392 | 0.0759 | 0.1319 | 0.2329 |

At the BAD view's 16.3 view px per degree, 27 directions of evidence leave
**1.13 view px** on the real field and **2.79** on the test's, so the test makes
the point a fortiori. The delivered measurement in 9.1 read about two view px,
which sits between them.

**The sweep is not monotone in coverage.** On the test's field 64 and 48
directions read 0.0677 and 0.0676 rms while their worst entries go 0.1403 to
0.1440. What is left depends on where the arc sits against the field's own phase
as well as on how wide it is, so neither column is a curve to read a threshold
off; what they establish is the difference between a ring with evidence
everywhere and a ring that is an arc.

**And the derivation's linearity has one caveat.** `T + fit(L - T)` minus
`fit(L)` is exactly `T - fit(T)` only if `fit` is the same linear operator in
both arms. `Along::fit` weights each direction by its own `off_conf`, which is
the smoothed correlation peak and therefore a function of the readings, so a
table that moves where the correlation lands can move the weights too. Both
columns above are computed at fixed weights. The delivered measurement of 9.1
carries whatever the weights actually did, and it is the larger number.

**A session's ring is an arc**, because only the directions with content
correlate; the table is a field over the whole circle; and where the ring has no
evidence the fit is unconstrained, the ridge pulls it to zero, and the table's
own value is delivered whole. That is why GOOD - where the band had evidence at
the crossing azimuth - was unchanged and BAD was not.

**This binds any future use of the `Table` vehicle.** Reading through it is not
enough on its own; what applies a table has to answer for `T - fit(T)` at the
directions the session never reads.

### 9.3 The owner's blind verdicts

Two builds, one clip, one view, no labels, four views, arms randomized
(`~/kjerag-ab/seam-ab.sh`). Verbatim:

> **"same, both bad"**

at every view, and the `main` arm called slightly steadier at the shimmer view -
which the instrument agrees with: 10.081 against 10.214 codes per frame over 60
frames. **He was right**, and the acceptance battery that said otherwise was
measuring a picture the app does not draw.

### 9.4 What is left, and the new rule

- The **per-frame trim** stays. It is the estimator finding and it demonstrably
  cleans the readings a fit is made from (8.1).
- The **field is measured, guarded and stored**, and nothing applies it. What is
  kept is `seam::along_terms`, `seam::along_kept`'s guard and
  `SeamSample::along_deg`. Why keep it dormant: the one regime 9.1 does not
  cover is the **first frames of a session**, before the band has any evidence,
  which is exactly where `T - fit(T)` is largest and where a stored field is the
  only thing that could act; and nine captures at a density the app does not
  reach is what the measurement cost. A pool fills over months, so a harvest
  that starts now is what a later attempt would have to start with. Nothing
  reads it, and the docstring at the storage site says what any reader must
  prove first.
- The **pool is not discarded**. An earlier form of this change discarded it,
  because samples stored under the old estimator are a worse estimate of the
  same pose and a pool that answers by agreement lets them outvote new ones.
  What paid for that cost was the applied field, and without it the ledger is a
  certain owner cost - a cold first file, five files to re-earn - against a
  benefit the band already covers wherever it has evidence.
- **`band::Table` ships at `REST`**, as on `main`, and the compute pass's
  read-through is removed with the thing it existed for.

**THE NEW BINDING RULE.** Any change that applies something at the seam must
include a **delivered-app-path comparison against `main`** in its acceptance,
not only the unbent instruments. The A/B protocol is part of the battery and not
only the owner's gate. Section 8's numbers were true in their domain; the domain
was the wrong one for an applied claim, and no amount of held-out rigour inside
it would have caught this.

**And the comparison has two halves, because one of them is not enough.**
`~/kjerag-ab/delivered.sh` runs both.

1. **Difference.** The app, at the view, photographed, against the same binary
   run twice. It says whether the two builds draw the same picture and how that
   compares with a build's own run-to-run spread. It **cannot say which is
   better**: it is a whole-window number dominated by whatever the two fits do
   to the framing, and it says nothing about the seam. A cross-arm difference
   under the control means "not resolved", never "identical" - one control pair
   is one sample and does not bound the spread.
2. **Quality.** `--bin step` and `--bin shear` with `seam=file` and **the band
   live**. These read the seam itself in the delivered domain. It is the half
   that answered this stage's own open question (9.5), and it is the half a
   difference metric cannot stand in for.

The capture in half 1 is only comparable if the fit **landed before the
shutter**: an empty pool fits off the file and walks the correction in over a
second, so a frame grabbed mid-walk is a picture of the walk. The script asserts
the fit's own report line rather than trusting the settle.

### 9.5 What the restructured branch delivers

The render path is now `main`'s: `Table::REST`, no read-through, no composition.
The only thing that can still differ is the **fit**, which the trim changes by
design. Both builds, empty pool, the app at each view, photographed after a 14
second settle, against the same binary run twice as the control:

| view | main vs main (control) | main vs branch |
| --- | ---: | ---: |
| GOOD | 4.443 codes mean, worst 74 | **2.275, worst 60** |
| BAD | 6.740, worst 234 | **7.343, worst 242** |
| shimmer | 0.105, worst 7 | **0.758, worst 30** |

At the two May-01 views the branch is **inside the same binary's own run-to-run
spread**. At the shimmer view it is outside it by about seven times, and the
report line says why: on that file the trim moves the fit by +0.032 deg of roll,
-0.079 of pitch and -1.31 px of `cy`.

**Which fit is better, answered in the delivered domain.** A whole-window
difference cannot say, and that was this section's open question until the
second half of the comparison was run: `--bin step` and `--bin shear` with
`seam=file` and the band live, both builds. Three signals, all one way
(the adversarial reviewer's runs, three each and deterministic):

| delivered, band live, `seam=file` | main | with the trim |
| --- | ---: | ---: |
| step at the seam, view px | -21.19 | **-18.89** |
| the band's along-seam load, mean deg | 0.176 | **0.159** |
| the same, worst deg | 0.792 | **0.498** |

and `--bin shear`'s residuals are smaller at all four bands with the steadiness
unchanged. Reproduced here independently, at the **same aim** - the registry's
step view, `VID_20260714_193252_00_006.insv time=2.836 yaw=111.83 pitch=4.12
fov=20.00 lock=1`, character for character - and from a **different band
state**: this run gave the band no warm-up and read 26 of 128 directions with
evidence where the reviewer's `warm=6.0` read 47 to 48. Step at the seam
**-21.97 to -20.69 view px**, its along-seam part -0.439 to -0.384 deg, and the
band's along-seam load **0.227 to 0.199 deg** mean and **0.613 to 0.538** worst.

That the absolute numbers differ and the direction does not is the **stronger**
reading, not the weaker one: the same aim seen through two band states, one with
half the ring's evidence of the other, moves the same way.

**The trimmed fit is the better one in the picture** - it leaves less step at
the seam and less for the per-frame band to carry, which is what a cleaner pose
should do - **and the claim is exactly as wide as its evidence**: one camera,
one flight (the July-14 X4 Air capture), two views on it, two band states. The
two May-01 crossings cannot be read this way at all - `--bin step`'s line fits
there come out at 51 to 54 px rms, which is the condition the registry warns
about before any step is quoted - and the ONE X2 view answers "no horizon
fitted on both sides of the seam". So this is the delivered-domain evidence for
the one thing this PR still applies, on the one flight that could carry it, and
it is the half a difference metric could not have supplied.

## 10. Across the seam: a research term, and what the delivered picture did with it

**This section is a research result and nothing here ships.** `KJERAG_EPI_TERM`
is off by default, the picture with it off is `main`'s byte for byte, and the
verdict below is that the pooled form of the term is not the fix.

### 10.1 The defect, and why a table on this axis is a different animal

Everything above is about the **along-seam** axis, where a leftover is the
camera by construction because no distance can reach it. Across the seam a
leftover is the camera plus the scene, and the band answers it per frame per
direction. That is why 6 called the across-seam axis untouched by design.

What reopened it is what the band does with a reading it cannot remove. The
epipolar bend is shared across the handover: `Reframe::blend_bent` gives each
lens the OTHER one's weight times the disagreement, so the two lenses are one
whole disparity apart everywhere in the corridor and the picture is right where
the disagreement is parallax. Where the disagreement is a **pose error** the
same arithmetic draws far content with a bend in it - zero at the corridor's
edge, half the disagreement at the contour, and the other half the same way on
the other lens. At the registry's BAD May-01 crossing that is a horizon drawn
as a shallow S.

`--bin epiramp` measures exactly that, in the delivered domain and nowhere
else: the app's own path with the band live and warm, photographed twice at two
handover widths, the far content correlated between them along the seam normal.
The 0.1 degree render is the reference because each side of it is drawn by one
lens with no ramp on it, so the lag between the two renders at one distance
from the contour is what the handover put there and nothing else.

### 10.2 What the term is

`seam::epi_term` composes, per direction of the band's ring:

- **the pose's own across-seam displacement**, `seam::moved`'s second component
  between the factory calibration and the pose being drawn. Pure geometry from
  the five knobs, no reading in it, and it is large: the fit is made to null the
  along-seam axis and nothing in it asks what it does to the other one, which on
  this camera reaches **2.5 degrees**.
- **the pooled static reading**, `EPI_STILL_DEG`: what the six X4 Air flights
  read across the seam under the factory calibration, trimmed per azimuth, from
  `docs/research/epi/epi-leftovers-x4.csv` on `research/epi-study`. Pose
  invariant, which is the only reason six flights' readings are six
  measurements of one thing.

Their sum is what the two lenses still disagree by under the drawn pose, and it
is applied as a displacement of lens 1's **whole picture** across the seam,
before projection, on the unwarped ray - one displacement instead of a ramp.
The band's own measurement pass reads lens 1 through it, so what the band then
fits and applies is what the term still leaves.

**It cannot fold**, for the along-seam term's reason read one axis over: the
displacement is across the seam and its gradient is along it, so the Jacobian it
adds is off-diagonal and its determinant stays exactly 1
(`the_across_seam_term_displaces_lens_one_across_the_seam_and_nowhere_else`).

### 10.3 The corpus: one seam crossing per flight

The two May-01 crossings are the registry's. The rest were derived for this
measurement on `research/crossing-views`, at this build, by tracing the 50/50
contour on a `fov=250` render and reading where it crosses elevation zero. The
derivation recovers the registry's own May-01 pair to about a degree
(-79.9 and +100.0 against -80.28 and +101.13), which is what validated it. Every
`lock=1` yaw here is a per-build quantity - the world-fixed lock's zero is the
heading the file opened on - so none of these lines may be copied to an older
commit without `--bin carried`.

| flight | line (`lock=1`) | what is at the seam |
| --- | --- | --- |
| May 01 GOOD | `time=50.117 yaw=-80.28 pitch=0.06 fov=55.69` | the registry's matched crossing |
| May 01 BAD | `time=50.117 yaw=101.13 pitch=0.75 fov=62.79` | the registry's mismatched crossing |
| Apr 10 | `time=45.112 yaw=-86.05 pitch=3.18 fov=38.28` | floodplain, meandering river, distant ridge; nothing near |
| May 26 | `clip 2/… time=600.000 yaw=-96.74 pitch=0.00 fov=58.00` | ploughed field, distant treeline, sun 25 deg off the crossing |
| Jul 14 | `time=600.000 yaw=156.90 pitch=0.00 fov=58.00` | treeline horizon over a wooded valley |
| Jul 14 (shimmer) | `time=36.303 yaw=162.31 pitch=5.44 fov=20.00` | the registry's motion view, on the same flight |
| Aug 02 | `clip 1/… time=600.000 yaw=135.58 pitch=0.00 fov=58.00` | corn rows to a treeline horizon; the tightest reading in the set |

**Two captures are their own rows and are never pooled with the six.** The
July-25 flight (`time=200.000 yaw=-127.45 pitch=0.00 fov=58.00`) is above a
**solid undercast for its whole length** - 200, 600, 1000 and 1500 s all are -
so its far content is a cloud top with the sun five degrees above the crossing,
which is a different target class from a treeline. The October ONE X2
(`time=270.000 yaw=-58.99 pitch=0.00 fov=58.00`) is a **different camera and a
ground capture**: the far stretch is a ridge crest down to mid-field, grass at 3
to 10 m crosses the same seam at the bottom of the frame, and the pooled static
table this term carries was measured on the X4 Air and not on it. The
registry's own X2 line is not a far-content crossing at all - the seam there
cuts a person 20 m off and the wing on the sand.

**One line the corpus refused.** April's registry pair has a mismatched half
(`time=43.143 yaw=93.36`) that reads the same order of disagreement as BAD, and
it is **not** usable as a far-field crossing: the prop cage and its netting
cross the seam over the last quarter of the arc and the first third is
low-contrast haze, leaving one narrow band of far content. `--bin step` fits no
horizon on its far side (rms 94.91) and `--bin crossing` withholds its gate.

**And one thing the corpus cannot do, which bears on the verdict.** The large
disagreement the BAD crossing carries sits at body arc +40 to +70, and on this
rig that half of the seam circle is where the pilot and the prop cage are. The
only other crossing measured in that band is April's refused line, whose narrow
far-content core reads the same order (-13.6 to -14.6 source px against BAD's
-13.37). Every other flight's clean far-field crossing is elsewhere on the ring
and reads between 0.06 and 5.87. So **"the other flights do not show the
defect" and "the other flights do not sample the arc where the defect lives"
are not separated by this corpus**, and no verdict below may be read as if they
were.

### 10.4 The calibration every row is taken at

The pose the app **draws** and not the registry's literal. The X4 Air rows are
all at the pool's own answer for camera `d8a393389b7b8639`,
`roll:0.795 yaw:-2.310 pitch:-0.936 cx:-3.28 cy:-11.91`, which is also the pose
the epi study's leftovers are quoted under. The registry's
`roll:0.577 … cx:-9.53` is a knob median that nothing draws, and a first pass of
this whole measurement taken at it has been discarded rather than reported.
The X2 row is at its own camera's pooled answer,
`roll:-2.426 yaw:1.114 pitch:2.562 cx:1.58 cy:-9.70`, **which is five identical
samples of one capture** - one fit wearing a pool's clothes (issue #156's
duplicate shape) - and its row says so.

Both arms of every pair share that calibration. The toggle is the only
difference between them.

### 10.5 The null

The toggle off draws `main`'s picture byte for byte, in both domains, at the
BAD crossing under the drawn pose:

| render | `main` at 75a03cc | this branch, toggle off |
| --- | --- | --- |
| `--bin reframe`, the unbent projection | `783182e2f9169b34e002690f1678b5e3` | the same |
| `--bin step`, the delivered picture with the band live and warm | `90b0b0354c3e2a34b3259d44d303e7e3` | the same |

Both checkouts built into their own `CARGO_TARGET_DIR`, which is not optional
(AGENTS.md, issue #47: one shared target directory silently gives an instrument
the other tree's binary).

The delivered one is the load-bearing half. The unbent render never runs the
band's compute pass, which is where the term's read-through lives, so a
read-through that fired with an empty table would not show there.

### 10.6 The instrument

`--bin epiramp`, and it reads the delivered domain and nothing else, which is
9.4's rule. `--bin reframe` draws the unbent projection and `--bin crossing`
builds its map with the band held off; both were the wrong domain for an applied
claim, and PR #167 is the record of what that cost.

One run is the app's own path: `Scene::still`, every frame from `warm` seconds
before the view played through `ScenePipeline` so the band is as warm as it is
in a real run, then the picture. Two runs make a reading, at two handover
widths. At `KJERAG_HANDOVER_DEG=0.1` each side of the seam is drawn by one lens
with no ramp on it, so the lag between the two renders at a given distance from
the contour is what the handover put there and nothing else.

**Each arm reads against its OWN cut render.** A build that displaces lens 1's
whole picture displaces it in the cut too, so a lag read that way is the
**residual** ramp - what the corridor still does after the displacement - which
is the question. Reading one arm against the other's cut would measure the
displacement instead, which is not a defect and is not what anyone sees.

The seam's own field is the app's, per pixel: the angle off the body's `xy`
plane, from `Reframe::body_ray`, and the unit normal from its gradient. So a
strip at "1.6 degrees off the seam" is that everywhere in the frame rather than
a straight line fitted through a picture. Lags are read at 0.7 to 4.8 degrees
either side - the epi-probe's 40 to 260 view pixels at the BAD crossing's scale,
in the unit that means the same thing at every field of view in the registry -
and a straight line is fitted through them and extrapolated to the contour.

**What the number is.** `Reframe::blend_bent` gives each lens the other one's
weight times the disagreement, so at the contour the two are half of it each and
in opposite directions. The swing across the whole corridor is therefore twice
either side's own, and `E` is lens 1's side doubled. Lens 0's is printed beside
it with its own rms, and where the two disagree the rms says which line
describes its own points. **A row whose rms is a large fraction of its own
intercept is not quotable**, which is `--bin step`'s rule in this instrument.

**The controls.** `plant=<view px>` slides the reference's lens 1 side by a
known amount before correlating: a probe that cannot see a shift it was handed
is not a probe. Every CSV carries the build, the file, the aim, both
environment variables, the reference it read against, the plant, the scale and
the window, because a CSV nobody can tell the provenance of is a CSV nobody can
check.

### 10.7 The `pose` arm takes the band's own eyes out, and it is measured

`KJERAG_EPI_TERM=pose` applies the knobs' own across-seam displacement with no
pooled reading taking it back out. That displacement reaches **2.5 degrees**,
and the band's own epipolar search runs `FAR_DEG` to `NEAR_DEG`, **-1.2 to
+2.6**. So a term near the top of its range moves what the band is asked to
correlate outside the window the band can search in.

That is exactly what it does. `--bin step` at the BAD crossing, same file, same
aim, same drawn pose, band live and warm at 6 seconds:

| arm | directions with evidence | epipolar mean | worst |
| --- | ---: | ---: | ---: |
| off | **96 of 128** | 0.554 deg | 0.948 |
| `full` | **96 of 128** | 0.518 deg | 0.995 |
| `pose` | **64 of 128** | 1.172 deg | 1.939 |

The `pose` arm loses a third of the ring's evidence, and what still correlates
reads twice as far out - the term's own size rather than the camera's. A band
that has gone quiet draws a *steadier* picture without the geometry having got
any better, which is why that arm's ramp numbers are reported nowhere in 10.8.

The `full` arm keeps every direction the shipped build keeps, which is what
makes its rows readable at all.

**A constraint any later applied across-seam term inherits.** Whatever such a
term carries, the residual it leaves has to stay inside the band's own search
window, or the thing measuring the residual is measuring nothing. Nothing in
the code enforces that today.

**And a number worth sitting with.** With `full` on, the band's epipolar mean
across the ring goes **0.554 to 0.518 degrees**. The term took a fifteenth of
what the band reads. That is the whole story of 10.8 in one row, measured
somewhere else.

### 10.9 T - fit(T), and whether the band fights the term

The along field's failure was `T - fit(T)`: with a table applied and the band
measuring through it, the delivered correction is `T + fit(L - T)` against
`fit(L)` with none, and the two differ by exactly `T - fit(T)`. `Along::fit` is
five terms over the whole circle, so it reproduces `T` only where the session's
arc has evidence and delivers `T` whole everywhere else (9.2).

**The across-seam channel is not that shape, and that is the one structural
thing this term had going for it.** The band's epipolar channel is **per cell**:
no five-term fit, no ridge, no arc. Where a direction has evidence the band
reads the residual and applies it; where it has none it applies nothing. So
`T - fit(T)` is `T` at the directions with no evidence and **zero** at the
directions with evidence - and at the directions with evidence the term does
not cancel either, because `T` and the band's own answer are applied by
**different laws**: `T` displaces lens 1's whole picture, the band's answer is
ramped across the corridor. That is the entire mechanism this experiment rests
on and it is why the ramp moves at all.

**Does the band fight it?** Measured, at the BAD crossing, band live and warm:
the same **96 of 128** directions have evidence with the term on as with it off
(10.7). The band does not lose the seam, does not re-open the crossover, and
does not chase the term. What it does is read a slightly smaller residual:
epipolar mean **0.554 to 0.518 degrees**.

**Does the picture still stand still?** This is the along field's other failure
probe, the one the #167 review's B1 used to catch the GPU trim snapping.
`--bin shear` at the shimmer view, 90 frames, `warm=6.0`, band live, same drawn
pose - the frame-to-frame step of the **applied displacement** at four bands,
and the band's own state:

| | off | `pose` | `full` |
| --- | ---: | ---: | ---: |
| band state, frame to frame, 360 directions | 0.0444 deg rms | **0.0933** | **0.0392** |
| applied step rms / worst at -150 px | 0.0035 / 0.0102 | - | **0.0016 / 0.0050** |
| at the seam | 0.0129 / 0.0698 | - | **0.0037 / 0.0096** |
| at +60 px | 0.0995 / 0.2565 | - | **0.0116 / 0.0391** |
| at +150 px | 0.0113 / 0.0681 | - | **0.0025 / 0.0084** |
| frame pairs stepping over a view pixel at +60 | **21 of 87** | - | **0 of 87** |

**With the term on the picture is steadier than `main`'s at every band**, and the
one band where the shipped build steps over a view pixel on a quarter of its
frame pairs stops doing it entirely. The band's own state settles too, 0.0444 to
0.0392 deg rms.

That is the opposite of what the along field's applied form did, and it is the
one result here that argues for the mechanism rather than against it: taking a
disagreement out of the corridor and putting it into a displacement removes the
thing the corridor was breathing.

**The `pose` arm doubles the band's own frame-to-frame state**, 0.0444 to 0.0933
deg rms, which is the same arm going blind seen from the other side: a band with
a third of its ring refused is a band whose surviving directions swing.

**And at the shimmer view the band reads BETTER with the term on**: 128 of 128
directions with evidence either way, epipolar mean 0.550 to 0.503 deg and worst
1.884 to 1.520.

### 10.11 What a later attempt would have to be

Not this table, and the size of what it would have to be instead is measurable
from what is above.

The needed term at one crossing is the disagreement itself: `E_off` in degrees,
because `E` is the disagreement and the ramp is what the corridor does with it.
The term applied is `E_off - E_on` with the sign the intercepts carry. So the
gap between what the pooled table supplies and what the crossing wants is a
number this measurement prints, per flight, and it is the thing that sizes a
per-session refinement.

**The per-session form is the obvious next candidate and it is not free.** The
band already reads this quantity per direction per frame; what it does wrong,
for far content, is apply it as a corridor ramp instead of as a displacement.
So a term that took the band's own settled epipolar ring and applied it whole
to lens 1 - a feedback path rather than a stored table - would need no corpus
at all. Three things that would have to be answered first, none of them
answered here:

1. **It would be applying a NEAR-field measurement to the whole picture.** The
   band's epipolar channel is parallax where the content is near, and moving a
   whole hemisphere by a near object's disparity is a worse defect than the
   shear it removes.
2. **The feedback has to be shown stable.** `T` changes what the band reads,
   which changes `T`. Nothing here measures that loop.
3. **It inherits 10.7's constraint**: whatever is applied, the residual has to
   stay inside the band's own search window.

### 10.12 Why the pooled table cannot work, in one number from the study itself

The six flights' epipolar leftovers **disagree with each other at a given
azimuth by more than the pooled table's whole amplitude**. From
`docs/research/epi/epi-leftovers-x4.csv`, over the 67 azimuths where two or more
captures read:

| quantity | value |
| --- | ---: |
| per-azimuth spread across flights, median | **0.597 deg** |
| the same, worst | **1.531 deg** |
| the leftover's own rms over all readings | 0.295 deg |
| the pooled per-azimuth median's rms | **0.229 deg** |

A mean over a population whose members are two and a half times further apart
than the mean's own size does not reconstruct any member of it. That is a
statement about this corpus and this reduction and not about the camera: it is
consistent with the leftover being scene rather than camera (the study's own
reading), with it being per-session, or with the readings being too noisy at
this density. Nothing here separates those.

### 10.13 What still needs the owner's eye, and what this stage did not do

- **The owner's panel.** `scratch/epiramp/panel-bad-clean.png` is the BAD
  crossing off above on, cut from `--bin epiramp png=` renders with nothing
  drawn on them. `panel-bad.png` is the same crop from `--bin step` renders and
  carries that instrument's trace lines across the very stretch of seam the
  question is about; it is kept only because the band-evidence numbers came off
  the same runs. **No eye has been over either.**
- **No blind A/B was run.** 9.4's protocol has two halves and the owner's blind
  verdict is the gate on an applied claim. Nothing here is an applied claim, so
  the gate was not asked for - but that also means **no number in 10.8 has been
  seen by an eye**, and the along field's whole lesson is that an eye disagreed
  with a battery that was measuring the wrong thing.
- **The paused-window byte check was not run.** It is `scripts/uitest.sh`'s
  toast capture, and it is the probe that caught the GPU trim's snapping. With
  the toggle off the picture is `main`'s byte for byte so there is nothing for
  it to catch there; with the toggle on nobody has looked.
- **One flight, one crossing.** Each row is one moment on one flight at one
  aim. The band's state is warm at 6 seconds and the reading is deterministic
  (the BAD row reproduces to the hundredth of a view pixel across runs), but a
  crossing is not a flight.

### 10.8 The delivered table

`E` is the swing the corridor delivers across itself, in view pixels of the
render the row was taken at, lens 1's side doubled (10.6). Every row is one
moment on one flight at one aim, band live, warm at 6 seconds, at the drawn
pose of 10.4, both arms sharing everything but the toggle. The reading is
deterministic: the BAD row reproduces to the hundredth of a view pixel across
separate runs of the whole sweep.

`l0` and `l1` are the two sides' own contour intercepts with the rms of each
line about its own points, because a row whose line does not describe its points
is not quotable and the reader has to be able to see that.

| crossing | E off | E on | | l0 off / on (rms) | l1 off / on (rms) |
| --- | ---: | ---: | --- | --- | --- |
| **May 01 BAD** | **19.94 px, 0.363 deg** | **14.70 px, 0.268** | **-26%** | -3.59 / -3.02 (10.3, 8.2) | -9.97 / -7.35 (1.7, 1.3) |
| **May 01 GOOD** | **1.98 px, 0.031 deg** | **11.32 px, 0.179** | **+472%** | +1.99 / +2.46 (0.3, 0.4) | +0.99 / +5.66 (1.9, **4.2**) |
| Apr 10 | 1.72 px, 0.018 deg | 3.61 px, 0.037 | **+110%** | +1.91 / +4.53 (0.3, 0.8) | +0.86 / +1.80 (1.4, **3.3**) |
| May 26 | 1.73 px, 0.029 deg | 0.52 px, 0.009 | -70% | -0.66 / -0.02 (0.1, 0.0) | -0.86 / -0.26 (0.4, 0.1) |
| Jul 14 | 8.99 px, 0.149 deg | 0.95 px, 0.016 | **-89%** | +3.63 / +0.27 (0.3, 0.1) | -4.50 / +0.48 (0.4, 0.2) |
| Aug 02 | 1.66 px, 0.027 deg | 0.74 px, 0.012 | -55% | -0.73 / +0.19 (0.2, 0.5) | +0.83 / -0.37 (0.1, 0.1) |
| Jul 14 shimmer | 5.47 px, 0.029 deg | 2.65 px, 0.014 | -52% | **-13.90 / -5.12** (2.0, 0.8) | +2.74 / +1.32 (**12.4**, **5.1**) |
| Jul 25, cloud top | 10.85 px, 0.180 deg | 4.74 px, 0.078 | -56% | +8.20 / +4.93 (0.9, 0.5) | -5.43 / -2.37 (3.3, 1.7) |

| **Oct 18, ONE X2** | 3.55 px, 0.059 deg | **3.55 px, 0.059** | **0%** | +0.83 / +0.83 (0.5, 0.5) | -1.78 / -1.78 (3.2, 3.2) |

**The X2 row is a zero and it is the guard working.** Both arms produce the
same CSV to the last digit, which means the composed term came out
`Table::REST` on that camera and the two builds drew one picture. `epi_term`
refuses a table whole rather than in part - whole support or nothing, the rule
stage 9 wrote after stage 5 scalloped - and on the X2 either the sum exceeded
`EPI_LIMIT_RAD` or a direction of the ring left a lens's picture. **Which of the
two is not established here**, and nothing in the run says so out loud, which is
a gap in the instrument rather than a result. What the row does establish is
that no X4 Air table reached the X2's picture, which is the right outcome for a
table measured on another camera and the wrong way to have got it.

Bold rms is a line that does not describe its own points and a column that
should not be quoted. Three rows carry one: BAD's lens 0 side at both arms
(near content in that stretch, which is why the run windows it), GOOD's and
April's lens 1 side **only with the term on**, and the shimmer view's lens 1
side at both arms - that view is `fov=20`, where 4.8 degrees off the seam is
most of the frame and the outer samples have nowhere to sit. At the shimmer
view the quotable side is lens 0, and it reads **-13.90 to -5.12 view px, a 63
percent fall**.

### 10.10 The verdicts

**(a) It does not collapse everywhere.** The bar was under about 4 view pixels,
the GOOD crossing's own floor. Four of the six X4 Air crossings land there
(May 26, Jul 14, Aug 02, and the shimmer view on its quotable side). **BAD does
not**: 19.94 to 14.70 view pixels, a quarter of the way, and 0.268 degrees is
still four times the GOOD crossing's own reading before anything was applied.

**(b) It collapses unevenly, and on two crossings it makes the picture worse.**
GOOD goes **1.98 to 11.32 view pixels** and April **1.72 to 3.61**, and on both
the lens 1 line stops describing its own points at the same moment (rms 1.9 to
4.2, and 1.4 to 3.3), which says the term has put a shape into that side that a
straight ramp no longer fits.

**And GOOD is the one that matters most, because of what it is paired with.**
The acceptance battery's first rule is *improve both May crossings without
trading one for the other; a field that fixes one at the other's expense is the
defect moved rather than removed*. This term improves BAD by a quarter and
costs GOOD five and a half times its whole reading, on **the same instant of the
same file**. That is the trade the rule names, and it is refused on it alone.

**(c) It fails on a flight that is one sixth of the table.** May-01 is not held
out - it is one of the six captures the pooled table is built from - and the
table still leaves three quarters of BAD's ramp in the picture and multiplies
GOOD's. 10.12 is the reason in one number: the six flights disagree at a given
azimuth by 0.597 degrees at the median and 1.531 at worst, against a pooled
table whose own amplitude is 0.229 rms. **The pooled static form of this term is
refused.**

**What is NOT refused, and this matters for what comes next.** The mechanism
works. The sign is right at every crossing but one, the term moves the delivered
ramp by about what it carries, the band does not fight it (10.9), and where the
term happens to be near the flight's own answer the ramp goes to nothing:
**Jul 14 reads 8.99 to 0.95 view pixels, an 89 percent fall, on both sides, with
every line describing its own points.** A across-seam displacement of lens 1's
whole picture *is* the right shape for this defect. What is wrong is the number
being put in it.

### 10.14 The gates

`cargo fmt --all --check`, `cargo clippy --workspace --all-targets -D warnings`,
`cargo test --workspace` (41 suites, green, including two new tests: the term's
axis, size and determinant on the projection, and that the composed table is the
pose plus the pooled reading and not something else) and
`scripts/name-check.sh`.

`scripts/uitest.sh` on real footage reads **47 checks, 2 failed**:
`ctrl+v goes back to the copied view`, which `main` has failed on this box since
before this branch (#167's own note), and `a toast is drawn clear of the
controls`. **Both are on `main` too**: the same harness run in a
checkout of 75a03cc, with its own `CARGO_TARGET_DIR` and its own app binary,
fails `a toast is drawn clear of the controls` as well. That control was run
because a branch whose delivered picture is `main`'s byte for byte cannot have
moved a toast, and a failure that had NOT been on `main` would have meant the
null was wrong somewhere the byte compare did not look. It was not.

## 11. The per-session arm: the same mechanism with the flight's own number in it

**10 stands as measured and this section does not rewrite it.** The pooled
static table is refused, and what refused it was the number rather than the
mechanism: the sign was right at every crossing but one, the band did not fight
the term, the delivered picture was steadier with it on at every band, and the
one flight that happened to sit near the pooled answer collapsed its delivered
ramp by 89 percent. 10.12 is why a pooled number cannot be near any flight's
answer: the six disagree at a given azimuth by 0.597 degrees at the median
against a pooled amplitude of 0.229 rms.

So this arm changes **one input**. `seam::epi_term` still composes the drawn
pose's own across-seam displacement with a reading; what the reading is is now
this capture's own, harvested off its own frames.

### 11.1 `--bin epifield`, and the three gates it needed

`seam::measure` reduces each direction's frames to one reading, which is what a
fit is made from and is the wrong granularity for a question about which
moments are near content. It is now `seam::moments` - the same walk, the same
acquisition, the same refusals - and `seam::measure` is that reduced. Neither
half's answer changed.

**The far gate is on the excursion and not on the reading.** Parallax on this
axis is one-signed: `band::Cell::metres` is `reach_m / disparity` and exists
only where the disparity is positive, because a negative one is not a distance.
So near content can only push a reading one way. A first pass takes each
direction's own middle; a moment whose excursion above that middle implies a
distance nearer than 60 metres, by `metres`' own arithmetic on this capture's
own baseline, is dropped; the survivors are reduced again by the trimmed middle.

**Applied to the reading itself the gate is nonsense, and that is measured.**
The factory calibration's own across-seam error reaches two and a half degrees
and is positive over half the ring, so an absolute gate at 60 metres throws away
**1829 moments of 3205** on the May-01 flight and calls a calibration a hedge.
On the excursion it drops **720**. The excursion is what a distance can move;
the middle is what the camera is.

**What the far gate cannot do**, plainly: the camera's own term and a far
object's parallax are the same sign on the same axis, and a session whose near
content never moves would have that content in its middle. This removes what
*wanders* nearby - the wing, the lines, the prop cage swinging through a
direction - and leaves the rest alone.

**A direction that departs from the ring's own five-term shape is refused.** The
factory across-seam error is pose-order on this camera, so a reading far off a
least-squares fit of those five over the whole ring is a correlation that found
the wrong feature: on the May-01 flight two directions read -1.86 degrees where
every neighbour and every other flight reads +2.36. A detector and not a
smoother - what survives keeps whatever it says above pose order, which is the
entire thing a per-session field exists to carry.

| session | directions read of 128 | moments | refused as near content |
| --- | ---: | ---: | ---: |
| May 01 | 88 | 3205 | 720 |
| Apr 10 | 104 | 2842 | 986 |
| May 26 | 123 | 4713 | 1280 |
| Jul 14 | 121 | 2307 | 816 |
| Jul 25 | **74** | 2974 | 820 |
| Aug 02 | 121 | 2492 | 814 |
| Oct 18 X2 | 119 | 6761 | 2438 |

The July-25 flight reads the thinnest ring in the corpus, which is what a
capture above a solid undercast for its whole length should read.

### 11.2 Three constraints, each from something 10 measured

**One: the band's own search window binds, and it is now code.**
`EPI_LIMIT_RAD` was a plausibility guess at three degrees; it is one degree now,
because the band re-measures the residual THROUGH whatever is applied, its
epipolar search runs -1.2 to +2.6 degrees, and a term at 2.5 degrees was
measured to take the band from 96 of 128 directions with evidence down to 64
(10.7). The `pose` arm is refused by it, which is the right answer, and
`the_across_seam_term_is_refused_when_the_band_could_not_re_measure_through_it`
is that as a test rather than as a paragraph.

**And a refusal says so, in one line.** That was the instrument gap the X2 row
exposed in 10.8: two arms produced identical CSVs to the last digit and nothing
in either run said the table had been thrown away. Now it does, and it says
which bound and by how much.

**Two: an unread direction is identity, and identity is on the COMPOSED TERM.**
This is a trap worth naming because the first draft of this arm fell into it and
the delivered measurement caught it. The term is the reading **plus** the drawn
pose's own displacement, so a direction with a zero *reading* still draws the
whole pose arm - two and a half degrees of it - which is the arm that blinds the
band. The May-01 field has 40 directions with nothing in them, and composed that
way the term reached **4.141 degrees** and was refused whole. `Session`
therefore carries a moment count per direction and `seam::supported` zeroes the
whole term where there is none, with a raised cosine over four cells walking it
in. Nothing is filled from a neighbour's value.

**Three: whole support or nothing, over the arc the session claims.** A field
whose composed term passes the bound over its own supported arc is accepted; one
that does not is refused entire rather than clamped, because a clamped field is
a different field from the one that was measured and nothing measured that one.

### 11.3 The three-column delivered table

Same nine crossings, same instrument, same drawn pose, each arm against its own
cut render. `E` is the corridor swing in view pixels, lens 1's side doubled.

| crossing | off | pooled | **per-session** | |
| --- | ---: | ---: | ---: | --- |
| **May 01 BAD** | 19.94 | 14.70 | **0.89** | **-96%** |
| **May 01 GOOD** | 1.98 | 11.32 | **0.35** | **-82%** |
| Apr 10 | 1.72 | 3.61 | 1.72 | field **refused at 1.104 deg** |
| May 26 | 1.73 | 0.52 | **0.68** | -61% |
| Jul 14 | 8.99 | 0.95 | 8.99 | field **refused at 1.582 deg** |
| Aug 02 | 1.66 | 0.74 | 1.66 | field **refused at 2.155 deg** |
| Jul 14 shimmer | 5.47 | 2.65 | 5.47 | the Jul-14 field, refused |
| Jul 25, cloud top | 10.85 | 4.74 | **0.57** | **-95%** |
| Oct 18 ONE X2 | 3.55 | 3.55 | **2.26** | -36%, its own session's field |

**The bet came in on every crossing whose field was accepted, and the trade is
broken.** The two May-01 crossings are the same instant of the same file and one
field serves both: BAD **19.94 to 0.89** and GOOD **1.98 to 0.35**, both under
the GOOD crossing's own perceptual floor, where the pooled table improved BAD by
a quarter and cost GOOD five and a half times its reading. That is the
acceptance battery's first rule satisfied rather than traded, and it is the
thing 10 refused the pooled table on.

Every accepted row improves and none worsens. The July-25 cloud-top row, read on
its own terms, goes **10.85 to 0.57**. The ONE X2, a different camera with its
own session's field where the X4 Air's pooled table had reached it not at all,
goes **3.55 to 2.26**. Both sides' lines describe their own points at every
accepted row (rms 0.03 to 0.61), which none of the pooled arm's improved rows
could say.

**And three fields of seven are refused whole by the band's own search window.**
April at 1.104 degrees, July-14 at 1.582, August at 2.155 - so those three
crossings draw the untouched picture and their rows are their `off` rows to the
last digit, which is the refusal being visible rather than silent. **The cost is
real and it is not hidden: July-14 is the flight the pooled table collapsed by
89 percent, and this arm gives it nothing.**

**The bound's exact value is a research choice sitting in an unmeasured gap, and
it should be said plainly.** What is measured is 0.7 degrees keeping 96 of 128
directions and 2.5 degrees leaving 64. One degree is between them and nothing
has been measured between them. Whether a field at 1.1 or 1.6 degrees blinds the
band is the obvious next measurement and it is not this one; **what is not
available is deciding it from the delivered table**, because a bound chosen to
let three more rows through would be a bound fitted to its own answer.

### 11.4 T - fit(T) with the per-session values, and one probe that could not be run

**The band keeps every direction it had, and carries a third of what it did.**
`--bin step` at the BAD crossing, band live and warm, May-01's own accepted
field:

| arm | directions with evidence | epipolar mean | worst |
| --- | ---: | ---: | ---: |
| off | 96 of 128 | 0.554 deg | 0.948 |
| pooled | 96 of 128 | 0.518 deg | 0.995 |
| **per-session** | **96 of 128** | **0.190 deg** | 2.011 |

That is the T - fit(T) statement in the delivered domain: the term took **two
thirds of what the band reads** and the band lost nothing to it, where the
pooled table took a fifteenth. The worst single direction goes the other way,
0.948 to 2.011, so there is at least one direction where the term overshoots
and the band is left carrying more than it started with; that direction is not
identified here.

**The steadiness half could not be run, and no number stands in for it.**
`--bin shear` is the instrument that caught the GPU trim snapping and that
measured the pooled arm's steadiness win (10.9). It refuses both May-01 views -
their seams lean too far off the rows, which is the instrument saying so and is
the same condition the registry warns about before quoting a step there - and
the one view it does read is the shimmer view, **whose flight's field this arm
refuses at 1.582 degrees**. So there is no view in this corpus where `--bin
shear` can read and a per-session field is applied.

**The pooled arm's steadiness win does not transfer.** It was measured on the
pooled term at the shimmer view and it is a fact about that term; nothing here
establishes that a per-session term is steady, and the whole lesson of this
stage is that a number measured in one domain does not carry into another. What
would settle it is a capture whose field passes the bound and whose seam
`--bin shear` will read, and this corpus has none.

### 11.5 The verdict, and what it is not

**The mechanism is confirmed and the source is settled: a session's own field is
what the term needs, and a corpus's mean is not.** On every crossing whose field
the guards accepted, the delivered ramp falls to the perceptual floor or under
it, both May-01 crossings improve from one field with nothing traded, and the
band ends up carrying a third of what it did.

**Three things this does NOT say.**

1. **It is not a shipping candidate.** Three fields of seven are refused by a
   bound whose exact value has never been measured, the steadiness half of the
   acceptance has no view it can be run on, and the offline harvest is a
   `--bin epifield` run per file that nothing in the app does or could do.
2. **No eye has seen any of it.** `scratch/epiramp/panel-bad-session.png` is the
   BAD crossing off above on, cut from clean renders. The owner's blind A/B
   against `main` is the gate this has to clear when it earns one, and 9.3 is
   the record of an owner disagreeing with a battery that was measuring the
   wrong picture.
3. **The far gate is a hypothesis with a delivered result behind it, not a
   proof.** Nothing separates a camera's own term from a far object's parallax
   at one azimuth; what the gate removes is content that *wandered*. That the
   result improves the picture is evidence the gate keeps mostly camera - it is
   not evidence that it keeps only camera.

**What a shipping form would have to answer**, in the order the evidence puts
them:

- **Where does the bound go?** 0.7 degrees keeps 96 of 128 directions and 2.5
  leaves 64; the three refused fields sit at 1.104, 1.582 and 2.155. Measuring
  the band's evidence against a planted term at those sizes is one afternoon and
  it decides three of nine rows.
- **Can the app harvest this itself?** `--bin epifield` reads 24 places by 6
  frames off a file before it draws anything, which is not something a player
  does. The band already reads this quantity live, per direction, per frame -
  what it does wrong for far content is apply it as a corridor ramp instead of a
  displacement - so a live form has no corpus problem and a feedback-stability
  problem instead.
- **Does it survive a moving picture?** Unanswered, and 11.4 says why.

### 11.6 The gates, and the paused window with the term applied

`cargo fmt --all --check`, `cargo clippy --workspace --all-targets -D warnings`,
`cargo test --workspace` (42 suites, green), `scripts/name-check.sh`.

`scripts/uitest.sh` **with a per-session field applied for the whole run**
(`KJERAG_EPI_TERM=session:…may01.txt`, on the May-01 file the field was
harvested from) reads **47 checks, 2 failed** - the same two `main` fails on
this box, and no others.

That is the **paused-window byte check** answered, and it is the probe worth
naming: the applied along-seam field's withdrawn form left 716790 pixels of
716800 differing between two captures of a paused window, against 10996 on
`main`, and this harness is what caught it. Nothing of the sort happens here.

The two failures are `a toast is drawn clear of the controls` and `ctrl+v goes
back to the copied view`, both of which a checkout of `main` at 75a03cc fails on
this box as well (10.14).

## 12. The blinding curve: it is not the term's size, it is the residual

11 refused three fields of seven on `EPI_LIMIT_RAD`, and that bound rested on
one observation: a 2.5 degree term took the band from 96 of 128 directions with
evidence down to 64. **That term was both large and wrong, and nothing separated
the two.** This separates them.

### 12.1 The method

Nothing can change the disagreement a capture actually has, so the sweep changes
the term instead. A gain `k` on a field that is **right** at `k = 1` gives a term
of `k|D|` and leaves the band a residual of `(k - 1)|D|`. So the pairs

- `k = 0` and `k = 2` carry the **same residual** at `|T| = 0` and `2|D|`,
- `k = -1` and `k = 3` carry the same residual again at `|D|` and `3|D|`,
- `k = -2` and `k = 4` do it once more,

and if the band's evidence tracks `|T|` the members of a pair differ, while if it
tracks the residual they match. The May-01 field is the one that is right: it
takes the delivered ramp at the BAD crossing from 19.94 view px to 0.89 and the
band's own epipolar mean from 0.554 degrees to 0.190.

`KJERAG_EPI_TERM=plant:<field>:<gain>` is that, and it is **exempt from the rail
and says so on every run**, because putting a term past the bound on purpose is
the entire experiment. `--bin step` at the BAD crossing, band live and warm.

### 12.2 The curve

| gain | term, worst | residual | directions with evidence | epipolar mean |
| ---: | ---: | ---: | ---: | ---: |
| -2 | 1.870 deg | 3D | **66** of 128 | 0.676 deg |
| -1 | 0.935 | 2D | **79** | 0.692 |
| -0.5 | 0.468 | 1.5D | **89** | 0.662 |
| **0** | **0** | **1D** | **96** | **0.554** |
| 0.5 | 0.468 | 0.5D | **96** | 0.361 |
| **1** | **0.935** | **0** | **96** | **0.190** |
| 1.5 | 1.403 | 0.5D | **96** | 0.267 |
| 2 | 1.870 | 1D | **95** | 0.432 |
| 3 | 2.806 | 2D | **95** | 0.823 |
| 4 | 3.741 | 3D | **91** | 1.134 |
| 5 | 4.676 | 4D | **83** | 1.171 |

**Read the pairs.** At residual `1D`: `k = 0` keeps 96 with no term at all and
`k = 2` keeps **95** with a term of **1.870 degrees**, nearly twice the bound
that refused three fields. At residual `2D`: `k = -1` keeps **79** at 0.935
degrees - *inside* the old bound - and `k = 3` keeps **95** at 2.806. At residual
`3D`: `k = -2` keeps **66** at 1.870 and `k = 4` keeps **91** at 3.741.

**In every pair the larger term keeps more of the ring.** `|T|` is not what
blinds the band. It is not even weakly what blinds the band: the correlation
between the two runs backwards.

**And the asymmetry is the band's own window.** The search runs `FAR_DEG` to
`NEAR_DEG`, **-1.2 to +2.6** degrees. A negative residual runs out of room at
half the magnitude a positive one does, which is exactly what the two sides of
the table do: the negative gains fall away from `k = -0.5` while the positive
ones hold to `k = 3` and only give at `k = 4` and `k = 5`, where the residual
finally passes +2.6. **The hypothesis was that a wrong term blinds when the error
it leaves exceeds the window's near edge, and that is what the numbers say.**

### 12.3 What that changes, and the guard it does NOT provide

`EPI_LIMIT_RAD` is now **2.8 degrees**, the largest term the sweep measured to
leave the band's evidence intact, and its docstring says what it is: a rail
against a field that is not a calibration at all. The three fields 11 refused -
April at 1.104, July-14 at 1.582, August at 2.155 - are all inside the range
where a correct-sign term costs one direction of 128, and they were refused for
no measured reason. 12.4 re-reads those crossings.

**The safety question is not `|T|` and it never was: it is `|T - truth|`, and
nothing knows `truth` before the band has measured through the term.** So there
is no compose-time test that can tell a right field from a wrong one, and no
static bound can be the guard. What can: a **staged walk-in** that applies the
term in steps, each small enough that the residual it leaves stays inside the
window even if the whole field is wrong, with the band re-measuring at each step
and the walk aborting when its evidence falls or its residual grows step over
step. The curve is what says that would work - a wrong step of walk size is
visible in the evidence count long before a wrong field of full size is - and it
is what says the abort has a signal to fire on.

**It is not implemented.** This section is the measurement that makes it the
right design and the record of it being the next thing, not a description of
something that exists.

### 12.4 The table with the three rows filled in

The crossings 11 refused, re-read with the rail on the quantity 12.2 measured.
Nothing else changed: same fields, same instrument, same drawn pose.

| crossing | its field | off | per-session, **before** | per-session, **now** |
| --- | ---: | ---: | ---: | ---: |
| Apr 10 | 1.104 deg | 1.72 | refused | **0.47** |
| Jul 14 | 1.582 deg | 8.99 | refused | **1.62** |
| Jul 14 shimmer | 1.582 deg | 5.47 | refused | **0.63** |
| Aug 02 | 2.155 deg | 1.66 | refused | **0.20** |

**Every one of them lands under the floor**, and the shimmer view - whose lens 1
line could not describe its own points at either arm before (rms 12.37 off, 5.06
pooled) - now reads 0.16 and 0.40 rms on the two sides. A row that was not
quotable became quotable, which is the ramp actually going away rather than a
number moving.

The coordinator's prediction was that July-14, the flight the pooled table
collapsed 89 percent, should reach the floor on its own field. It does: **8.99 to
1.62**.

### 12.5 The steadiness half, run at last

11.4 could not run it: `--bin shear` reads only the shimmer view and that
flight's field was refused. It is admitted now. `--bin shear` at the shimmer
view, 90 frames, `warm=6.0`, band live, same drawn pose:

| | off | **per-session** |
| --- | ---: | ---: |
| band state, frame to frame, 360 directions | 0.0444 deg rms | **0.0349** |
| applied step rms / worst at -150 px | 0.0035 / 0.0102 | **0.0016 / 0.0057** |
| at the seam | 0.0129 / 0.0698 | **0.0022 / 0.0059** |
| at +60 px | 0.0995 / 0.2565 | **0.0039 / 0.0106** |
| at +150 px | 0.0113 / 0.0681 | **0.0013 / 0.0038** |
| **frame pairs stepping over a view pixel at +60** | **21 of 87** | **0 of 87** |

**Steadier at every band, and the one band where the shipped build steps over a
view pixel on a quarter of its frame pairs stops entirely.** The band's own state
settles too. What the term leaves for the band to carry at those four bands is
-0.007, +0.026, -0.007 and +0.003 degrees, against -0.604, -3.479, +1.617 and
+2.534 with it off.

This is the half a difference metric cannot stand in for and the half that was
outstanding. It passes.

### 12.6 The nine-row table, final form

| crossing | off | **per-session** | |
| --- | ---: | ---: | --- |
| **May 01 BAD** | 19.94 px, 0.363 deg | **0.89 px, 0.016** | **-96%** |
| **May 01 GOOD** | 1.98 px, 0.031 deg | **0.35 px, 0.006** | **-82%** |
| Apr 10 | 1.72 px, 0.018 deg | **0.47 px, 0.005** | -73% |
| May 26 | 1.73 px, 0.029 deg | **0.68 px, 0.011** | -61% |
| Jul 14 | 8.99 px, 0.149 deg | **1.62 px, 0.027** | **-82%** |
| Aug 02 | 1.66 px, 0.027 deg | **0.20 px, 0.003** | **-88%** |
| Jul 14 shimmer | 5.47 px, 0.029 deg | **0.63 px, 0.003** | **-88%** |
| Jul 25, cloud top | 10.85 px, 0.180 deg | **0.57 px, 0.010** | **-95%** |
| Oct 18 ONE X2 | 3.55 px, 0.059 deg | **2.26 px, 0.037** | -36% |

**Nine of nine improve. Every one lands under the four view pixel floor.** The
two May-01 crossings are the same instant of the same file and one field serves
both. Two cameras, seven captures, a treeline, a cloud top and a beach.

For the record's sake, the pooled column those nine replaced: -26%, **+472%**,
+110%, -70%, -89%, -55%, -52%, -56%, 0%.

### 12.7 Readiness for the owner's blind A/B, and what is still missing

**What this now has** that 10 and 11 did not: nine of nine crossings improved
with none traded, the band keeping every direction it had and carrying a third
of what it did, the steadiness half run and passed, the paused-window byte check
answered, and a bound that is a measurement rather than a guess.

**What it still does not have, and none of it is small.**

1. **No eye has seen any of it.** That is the gate, and 9.3 is the record of an
   owner disagreeing with a battery that was measuring the wrong picture. The
   crops are `scratch/epiramp/panel-bad-session.png` and
   `scratch/epiramp/panel-jul14-session.png`.
2. **The staged walk-in is not implemented.** 12.3 is the design and the
   evidence for it; the rail at 2.8 degrees is not a substitute, because the
   quantity that decides safety is `|T - truth|` and nothing knows `truth`
   before the band has measured through the term. **A wrong field inside the
   rail would be applied whole**, and 12.2 measures what that costs: a
   wrong-sign term of 0.935 degrees takes 17 of 128 directions out.
3. **The harvest is offline.** `--bin epifield` reads 24 places by 6 frames off
   a file before anything is drawn. A player does not do that, and the form that
   could - the band already reads this quantity live, per direction, per frame -
   has the feedback-stability question instead, unmeasured.
4. **One flight, one crossing per flight.** Nine moments across seven captures.

**So the honest statement is:** the design question is answered - a session's own
far-gated field, applied as a whole-picture displacement, removes this defect -
and the engineering to ship it is the walk, the live harvest, and the owner's
eye, in that order.
