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
shrunk by a ridge. It reproduces `T` only where it has evidence. Planting the
real pooled field and fitting it back
(`a_partial_ring_cannot_fit_away_a_table_over_the_whole_of_it`):

| ring directions with evidence | `T - fit(T)` rms | worst |
| ---: | ---: | ---: |
| 128 of 128 | 0.0007 deg | 0.0011 deg |
| 64 | 0.0080 | 0.0175 |
| 48 | 0.0247 | 0.0514 |
| **27**, which is what `--bin step` reports on real footage | **0.0333** | **0.0696** |
| 16 | 0.0392 | 0.0759 |

0.0696 degrees at the BAD view's 16.3 view px per degree is **1.13 view px**,
the size and the variability-with-azimuth of what was measured. **A session's
ring is an arc**, because only the directions with content correlate; the table
is a field over the whole circle; and where the ring has no evidence the fit is
unconstrained, the ridge pulls it to zero, and the table's own value is
delivered whole. That is why GOOD - where the band had evidence at the crossing
azimuth - was unchanged and BAD was not.

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
-0.079 of pitch and -1.31 px of `cy`. **Which of those two fits is better in the
delivered picture is not established here**, and the two fits' own residuals do
not answer it because each is measured on its own reading set. That is the live
question this PR leaves for the owner and the reviewer.
