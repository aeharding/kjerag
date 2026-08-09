# The chromatic line: what to estimate, from what, applied where, and what refuses it

**Status:** design memo. Nothing is built, and the only pixels read this round
are the owner's Studio on/off pair (section 2), which is CPU-cheap and was
allowed so that the maker's own correction could be read for its ORDER. **The
measuring phase is section 7 and is the next run, not this one.** A perf study
owns the GPU.

**Date:** 2026-08-09. **Scope:** issue #103, stage 10's lead item. The
per-channel generalization of stage 3's pooled gain, done on the evidence this
time.

**What this memo is for:** six answers and a pre-registered acceptance, so that
increment 1 can be written as a build brief by somebody who did not read the
last four days.

---

## 1. The evidence base, and what each piece binds

Quoted, not re-derived. Each of these is a ruling or a measurement somebody
already paid for.

**The oracle.** `~/Videos/studio_onoff/chromatic_calibration/{off,on}.jpg`:
Insta360 Studio's own render of the owner's dirt reference, the one variable
being Studio's **Chromatic Calibration** toggle. That toggle ALONE closes the
seam on his reference, and the defect it closes is a **hue** step and not a
luminance one: an R minus G split of 5 codes on 11-code warmth, about 45 percent
relative, along the seam swoop in OFF, and 0.0 in ON. The valid instrument is
the **arm-internal discontinuity test** - the band region against its own
surrounding dirt, per channel - which is stretch-proof, and the Weber law it
serves is that dark content is judged relative.

**The refusal that scopes this line** (PR #177, docs/research/seam-blending.md
17 to 23, insv-format.md 6.3). There is **no per-lens photometric metadata**.
Shutter is per lens (records 4 and 12) but the two auto-exposure loops trade it
against a sensor gain no file records; record 9 is one track for the whole file;
no key of the record-1 protobuf is an exposure; the ONE X2 does not write record
12 at all; and both of an `.OSV`'s `djmd` tracks carry the same exposure block.
The metadata correction was built, measured on nine of the owner's own reference
views, and left **11 to 228 times** the artifact it was meant to remove.
**So the chromatic line MUST estimate from pixels**, which is the danger zone
stages 7 and 8 died in.

**Their binding lessons** (seam-blending.md 16, and the ANTI-ACCEPTANCE entry in
reference-views.md):

- **A field applied over an area is accepted ON THE AREA**, not on the boundary.
  The instrument is `--bin colour`'s interior block: main's one gain over the
  whole ring reads 0.03 percent of roughness, the rejected stage-8 build 1.01,
  planted ripples 2.07 and 8.27, a null 0.000.
- **Per-direction offsets over wide support paint noise**, which is stage 5's
  scalloping reborn on the photometric axis, and it is what the owner saw as
  dark streaks running away from the seam across his soil.
- **Never freer than the evidence.**

**The architecture on main @ 8887745.** Flat stitch, fusion, an anchored
handover line, and **no applied band corrections** except stage 3's pooled luma
gain. The band still measures every frame and what it measures reaches no pixel
(docs/research/studio-parity.md 3.1). And P.1's keeper finding, which is an
advantage here rather than the hazard it was there: **anything applied at fusion
is invisible to the band's compute, because the band samples the decoded
planes.** Section 5 spends that.

**The owner's standing photometric references** (reference-views.md): the dirt
views on the May file, the green-hue pair on the April file (sun-facing lens
cast, OPEN, gates stage 10), the hard-mode sun view (clip 1, glare and both
crossings and the sun in one frame), and the fov-249 DJI line. **Studio is the
bar** for mitigation-shaped defects (owner ruling, 2026-08-02).

---

## 2. The Studio cross-check: what their toggle actually is (question 6)

**This is the one piece of pixel work this round.** Both frames of the oracle
pair are 3840x2160 RGB. Everything below is block means, which is why a pair of
independent JPEG encodes does not carry the structure: 32 percent of pixels are
bit-identical between the two files and the per-pixel difference has a standard
deviation of 2.2 / 1.3 / 2.0 codes with a mean of -0.010 / -0.005 / +0.019.

### 2.1 The pair is a clean single-variable A/B, and its own far field proves it

| region, off levels R/G/B | ON over OFF, per channel | difference, codes |
| --- | ---: | ---: |
| top-left sky, 1000x400 px, 206.7 / 203.4 / 201.9 | **1.0001 / 1.0001 / 1.0000** | +0.015 / +0.016 / +0.005 |
| sky right of centre, 800x400 px, 142.6 / 156.8 / 174.5 | 1.0009 / 1.0007 / 0.9995 | +0.121 / +0.104 / -0.085 |
| dirt right of the seam, 1000x500 px, 31.0 / 21.3 / 19.8 | 0.9990 / 1.0001 / 0.9987 | -0.030 / +0.002 / -0.027 |

Away from the seam Studio changes **nothing**, to a ten-thousandth of a gain on
bright content. That is both the control that says the two exports differ by one
toggle and nothing else, and the first half of the finding.

### 2.2 What it does change: a corridor on the seam, antisymmetric across it

The same statistic near the seam, at the dirt end of the swoop:

| region, off levels R/G/B | ON over OFF | difference, codes |
| --- | ---: | ---: |
| the cool side, 600x300 px, 47.1 / 40.2 / 42.7 | **0.9054 / 0.9742 / 0.9174** | -4.46 / -1.04 / -3.52 |
| the warm side, 600x300 px, 23.7 / 18.2 / 18.8 | **1.2106 / 1.0347 / 1.2137** | +5.00 / +0.63 / +4.03 |
| the cool side, one cut along, 30.4 / 20.8 / 25.2 | 0.8687 / 0.9480 / 0.8765 | -3.99 / -1.08 / -3.11 |
| the warm side, one cut along, 23.2 / 17.1 / 18.3 | 1.1820 / 1.0278 / 1.1730 | +4.21 / +0.48 / +3.17 |

Three readings fall out of that table.

1. **It is antisymmetric across the seam**: one side is pushed down and the
   other up, by about the same amount. That is a symmetric split, and it is the
   same shape stage 3's `Tone::split` already applies for the achromatic term.
2. **It is chroma-dominant.** R and B move together and G barely moves, which is
   the green-magenta axis. Over every block whose darkest channel clears 40
   codes, the ON/OFF log gain has a standard deviation of **0.0059 in luma
   against 0.0098 in R minus G and 0.0116 in B minus G**: the chroma carries
   about twice what the luminance does.
3. **The support is compact and it dies out into both hemispheres.** The
   zero crossing traces one smooth monotone curve across the frame, from
   (0, 1740) to (3600, 84) in picture pixels, which is the seam swoop; the
   correction reaches zero about 500 to 700 px either side of it, and is
   0.0 codes at the frame's far corners.

### 2.3 And its size and its hue BOTH vary along the seam

Amplitude of the ON/OFF change in R minus G, walked along the swoop:

| where along the seam | amplitude, codes | the level it sits on | relative |
| --- | ---: | ---: | ---: |
| the dirt end, x 0 to 960 | -3.8 to -5.0 and +3.4 to +5.5 | 19 to 31 | **12 to 27%** |
| the middle, x 1200 to 1920 | -0.9 to -1.6 and +0.9 to +1.5 | 20 to 26 | 4 to 7% |
| the sky end, x 3120 to 3600 | -1.7 to -2.4 and +1.4 to +2.6 | 171 to 186 | **0.8 to 1.5%** |

And the chroma direction turns with it. At the dirt end R and B move together
against G. At the sky end they move **opposite** each other:

| the sky end of the seam, off levels | ON over OFF | difference, codes |
| --- | ---: | ---: |
| one side, 169.1 / 180.5 / 190.0 | 0.9845 / 0.9928 / **1.0069** | -2.63 / -1.29 / **+1.31** |
| the other, 152.8 / 167.3 / 187.8 | 1.0122 / 1.0029 / **0.9919** | +1.87 / +0.48 / **-1.52** |

### 2.4 The verdict on their order, and it is not the convenient one

**Studio's Chromatic Calibration is not constant-like.** It is not one gain
triple per lens: a gain triple of 0.905 / 0.974 / 0.917 would move 170-code sky
by 16 codes and it moves it by 2. It is not one offset field either: the size
varies along the seam by three times in codes and by twenty times relative. It
is not one chroma vector scaled by a scalar field, because the chroma direction
itself turns from a green-magenta axis to a blue-amber one along the same seam.
It is not a per-lens radial or shading field, because a shading field cannot be
zero at both frame corners and 5 codes in the middle of the same hemisphere.

**What is left is a per-position, per-channel, locally estimated correction with
compact support on the seam and a symmetric split.** In our vocabulary that is
at or above the order of stage 8's per-direction field over wide support, which
is the thing this campaign has been refused twice. It is also **smooth**: the
along-seam amplitudes above walk 3.8, 4.4, 5.0, 4.2, 2.8, 1.6, 1.3 without
oscillating, and the maps show no striping at 24 or 48 px blocks.

**So the oracle proves the defect and licenses the acceptance target. It does
not license the model.** Studio reaches 0.0 by a mechanism our own evidence bar
forbids at increment 1, and the difference between their field and stage 8's is
not the order, it is the estimation discipline. Copying the order without
copying the discipline is exactly how stage 8 ended.

**One thing it does license, and it is the sharpest constraint in this memo:**
whatever we apply, Studio's own answer changes **nothing** in the far field. A
hemisphere-wide constant changes both hemispheres by half the split. That is a
bigger change to his picture than the maker makes, it is defensible (if the
split is real then one hemisphere is wrong today, and stage 3's gain already
splits an achromatic difference the same way), and it is an **accepted tradeoff
that goes at the top of the PR body and gets relayed to him as a question**
(AGENTS.md, the 2026-07-31 root cause).

---

## 3. What is estimated (question 1)

**Two numbers per camera. A luminance-neutral per-channel gain triple per lens,
constant over the hemisphere, split symmetrically.**

### 3.1 The decomposition, which is what makes it two and not three

Let `d = (dR, dG, dB)` be the natural log of lens 1's channel over lens 0's on
the same content. Split it:

```text
achromatic   L = w . d            w = BT.709 luma weights
chromatic    c = d - L(1,1,1)     w . c = 0 by construction
```

`L` is **already owned and already shipped**: it is `Tone::log_gain`, fitted by
`pooled_gain`, smoothed at `TAU_GAIN_S`, clamped by `LIMIT_LN`, applied in
`picture()` as `tone_split()`. The chromatic line estimates and applies `c` and
nothing else.

What reaches the picture is a gain triple per lens, `exp(+c/2)` on lens 0 and
`exp(-c/2)` on lens 1, **normalized so that its own BT.709-weighted mean is
exactly 1**. Normalizing the multiplier rather than its logarithm matters and is
not fussiness: a triple that is neutral in log space is neutral in the space it
multiplies only to first order, and at the chroma sizes 6.11 measures the second
order is about one percent of luma, which is 2 codes on bright sky. Normalized
on the multiplier it is exactly neutral, by an equality a test can assert.

Three properties follow, and each is worth naming because each is a thing that
went wrong before:

- **It cannot double-correct with the pooled gain**, because it carries no
  luminance. No refit, no re-baseline. That is a claim with a control attached
  (section 6).
- **It cannot stripe**, because it has no spatial freedom at all. A constant is
  absorbed exactly by the zeroth term of the interior statistic's five-term
  harmonic. Say the caveat with the number, as P.1 §20 did: that pass is worth
  nothing as evidence, because the failure mode it guards is not the failure
  mode a constant has. What says a constant cannot stripe is the constant.
- **Its null is an equality.** When `c` is zero the triple is exactly
  `(1, 1, 1)` on both lenses, by a `match` and not by trusting `exp(0.0)`, which
  is the rule `Tone::split` already keeps and the reason main is byte-identical
  everywhere the mechanism is quiet.

### 3.2 Can two numbers close a 5-code split? The honest answer

**On the seam, yes, if the split is a hemisphere-scale property.** Five codes of
R minus G on 11-code warmth is a difference between what two lenses call the
same dirt. A constant per-channel gain per lens is exactly the model of "the two
lenses white-balance differently", and it nulls that difference at every
direction at once.

**Whether the split IS hemisphere-scale is not settled by the oracle and is the
first thing the measuring phase asks** (M1, section 7). Section 2 shows Studio
correcting locally, which is consistent with a hemisphere-scale split corrected
gently, and equally consistent with a purely local disagreement that no constant
can reach. The two are told apart by one measurement: the two lenses' own
per-channel far-field ratio round the ring. **If that ratio is inside the
instrument's noise, increment 1 is refused before it is built**, and this memo
says so in advance so that the refusal costs a day rather than a branch. That is
P.1's ending and it is the good ending.

### 3.3 Every added order, and the named evidence it would need

| order | what it would buy | the evidence that admits it | status |
| --- | --- | --- | --- |
| constant per channel per lens, 2 DoF | a hemisphere-scale white-balance difference | M1 finds a far-field per-channel ratio above noise that reproduces within a capture | **increment 1** |
| a gain and an offset per channel | a veiling-glare or black-level difference | M2 separates them over the ring's own 20-to-190 code range and the offset wins | phase 2, family change not order change |
| a per-lens radial per-channel field | per-channel vignetting and lateral colour | a per-channel ratio at fixed angle from the lens axis that reproduces **across five places in one capture** | **already refused once.** 6.11 ran that test: at one azimuth the field moves by 3 to 27 codes between five places in the same capture, so what varies is glare and not glass |
| per-azimuth, one cycle and two | 6.11's measured shape, worth a third to a half again on every capture and above noise on every one | the full stage-7/8 bar: interior coherence PER CHANNEL at main's 0.03 percent class on dark content, a held-out capture, and the owner's eye label-blind | **never without that bar** |
| a local seam-centred field, which is what Studio does | parity with the maker | out of scope here. That is the belt's neighbourhood, and the belt is one mode at a time behind its own switch (studio-parity.md 7) | excluded |

---

## 4. From what evidence (question 2)

### 4.1 The tone machinery's own pattern, per channel

Stage 3's estimator is already the right shape and it is worth reading before
changing it. `measure` correlates one patch per azimuth over 128 directions,
half the ring per frame; at the shift that made the two patches the same
content, `read_photometry` writes `Cell::tone = ln(sum1/sum0)` and
`Cell::lit = sum0/count`; `pool` fits one number over the far field, weighted by
each direction's trust and by `lit` squared, and eases `Tone::log_gain` towards
it at `TAU_GAIN_S`. It is a ratio of means and not a mean of ratios, because
what the correction inverts is the ratio of means. It is read and applied in the
video's own gamma-coded space, so no transfer function is assumed at either end.

The per-channel version is that, three times, with two changes.

**The chroma planes are already in the bind group.** The draw's group 0 carries
`luma0`, `chroma0`, `luma1`, `chroma1` at bindings 1 to 4; the band's compute
shader declares 1, 3 and 5 only, with its own comment saying why: *"a doubled
edge is geometry and geometry is in the luma, and a bind group may carry
bindings a shader has no use for"*, and, at `luma_at`, *"the chroma planes are a
quarter of the resolution the correlation wants"*. **Both reasons are about
correlation and neither is about a mean.** A photometric mean over 441 samples
does not care that the plane is a quarter resolution. Declaring
`@group(0) @binding(2)` and `@binding(4)` in the measure shader costs two lines,
no layout change, no new upload, and no change to the correlation, which stays
luma-only.

**The weighting must not be inherited, and this is the sharpest technical point
in the memo.** `lit` squared was chosen on measurement, and the measurement was
about the achromatic gain: three poolings over nine captures, and brightness
squared left the smallest step at the seam on all nine. It is also, measured,
**one of the three reasons stage 7 could not reach the artifact**: it puts the
directions where the defect is visible at about one percent of the weight
(seam-blending.md TL;DR). Dark content is where the owner's every rejection has
been. So the chromatic term's weighting is a decision of its own, and the memo
pre-registers it as a fork rather than a choice: fit the two chroma degrees of
freedom under `lit` squared, under equal weight, and under a Weber weight, and
report all three columns the way the `models` table did. What picks the winner
is the oracle's own statistic at the owner's dirt views.

### 4.2 Outlier discipline, inherited where it was earned

- The far-field cut, read as **the disparity the pass is drawing with** and not
  as the last reading a direction ever took. That was a real defect, found and
  fixed in 6.11, and it took the colour pool from 37 directions to 70 at the
  owner's reference.
- `CLIP_HIGH` and `CLIP_LOW`: a clipped patch leaves nothing to read, and a
  direction that reads nothing keeps what it had and loses the evidence that
  weighs it. Absence is not a reason to believe the opposite.
- A **runaway guard per channel, re-derived and not copied.** `LIMIT_LN` is 0.25
  because the achromatic ratio was measured at 0.946 to 1.004 over seven
  captures and the guard is the widest, times four. 0.25 ln of hue is 28
  percent, which is far outside anything 6.11 measured (spreads of 1.5 to 15.6
  codes; the corpus X4 with the sun in one lens, 10.3). Fit the chroma degrees
  of freedom over the corpus, take the widest, times four, and say the number.
- A level floor. A chroma ratio on 3-code content is noise, and the guard has to
  be stated rather than left to `lit` squared to imply.

### 4.3 What content refuses, and one refusal that is a problem

**Sky-only is refused today, and it is where the owner sees the defect.** The
band refuses a patch with under six codes of standard deviation, because flat
sky correlates with anything. On the nine captures measured that is **20 to 64
percent of the ring**. 6.11's own finding cuts the other way: what a photometry
needs from an alignment is proportional to the content's own gradient, so the
patch that is hardest to align is the **easiest** to read a colour on, and it
priced the misregistration at under a code against colour differences of 2 to
33.

So the chromatic estimator wants an acceptance rule of its own: *a direction may
be refused for the geometry and still be read for the colour, at zero shift,
where the residual the pass leaves is worth under a code of gradient.* That is a
change inside the band's compute pass and it is the largest single item in
increment 1's cost. **It is also not obviously required at increment 1**, and
the memo does not decide it: M5 measures what fraction of the ring today's rule
leaves at the oracle's own view, and the answer decides whether this ships now
or waits.

**Night and very dark content** refuse by the level floor and the clip guards,
and the ring's yield is reported rather than assumed.

**A one-lens file, a seam that has never correlated, and every frame before the
first reading** are exactly zero, by the same equality `Tone::split` uses.

### 4.4 Temporal behaviour: slower than the gain, not faster

The band's own reasoning applies twice over. It says a gain has more reason to
be smoothed hard than a bend does, because *"a gain that flickers changes the
brightness of everything, which is the one artifact worse than the step it is
correcting"*. A white balance that flickers changes the **colour** of
everything, and a hue flicker is worse than a static cast by the same argument
one step further along. Start at `TAU_GAIN_S`, which is `TAU_FAR_S`, and treat
anything faster as needing its own evidence.

The filter class is the **tone gain's**, not the anchor's: a first-order ease
towards the pooled reading, no events, no states, no thresholds. The anchor's
closed-form flow exists because the anchor has an allowance it can run out of
and a rail it must not slam into; a chroma term has neither. The one thing to
copy from the anchor is its seek rule: a discontinuity in the film is a seek,
and a seek re-seeds rather than easing across, which is what
`Correction::land` and the first frame of a file already do.

Pre-register the check, in the anchor's own units: the drawn chroma term's worst
frame-to-frame change over the owner's fast segment, against the floor of the
reading it is drawn from.

---

## 5. Applied where and how (question 3)

### 5.1 Fusion-side, one line, and what the invisibility buys

`picture()` in `crates/render/src/scene.rs` already reads
`let tone = tone_split();` and multiplies each lens's decoded RGB by a scalar.
The chromatic term makes that a `vec3` per lens. One change of type in a line
that already exists, on both twins.

**The loop stability argument, and it should be made in these words.** P.1
recorded as a hazard that *"anything applied in the fragment shader is invisible
to the band, because the band measures the source and not the picture"*, and it
was a hazard there because the applied term came from metadata and the loop
could never learn it was wrong. **For a term fitted from the same pixels the
band measures, that same property is the whole of the stability story**: the
estimator is a pure function of the decoded source, there is no feedback path,
there is no integrator, so it cannot oscillate, cannot wind up, and cannot chase
its own correction. It converges on the source's own per-channel ratio, and
applying half of it to each lens nulls that ratio in one shot rather than over a
settling time. The only dynamics in the whole mechanism are one ease and one
seek reset.

**The hazard that remains, stated plainly:** the pass cannot check its own work.
So the check is an instrument reading the DRAWN picture, and it is pre-registered
in section 6 rather than left to a loop.

### 5.2 Both twins, and the guard's blind spot

Every number the map is made of exists twice, once as Rust and once as the WGSL
that same file emits, and `crates/render/src/twin.rs` compiles the shipped
`projection::wgsl()` on the device and compares. **It does not compile the
band's shader.** P.1 put its arm in the projection uniform for exactly that
reason, and said so.

The chromatic term cannot take that escape, because unlike a metadata ratio it
is **measured on the GPU** and its home is the state buffer beside
`Tone::log_gain`. Two ways out, and the memo recommends the first:

1. **Extend the guard.** `twin.rs` already compiles a WGSL string with a probe
   entry appended; doing the same for `band::CELL` plus `LOOKUP` and comparing
   `tone_split()` against `Tone::split()` over planted buffer states is roughly
   sixty lines and closes a blind spot that exists today, for the achromatic
   gain as much as for the chromatic one.
2. Read the band back once a frame on the CPU and write the triple into the
   projection uniform. That buys the existing guard and costs a per-frame GPU
   readback on the display path, which is the class of thing `stall.rs` exists
   to catch. Not recommended.

Either way, two WGSL-only mutations get planted and each has to be the only
failing test in the workspace, which is the standard both #176 and P.1 met.

### 5.3 Interaction with the pooled luma gain: none, and it is testable

By construction (3.1) the chromatic triple carries no luminance, so there is
nothing for the pooled gain to double-correct. **No refit and no re-baseline is
planned, and that is a claim rather than an assumption**, so it gets the control
P.1's finding hands us for free: the pooled gain reads `+0.00287` ln at evidence
0.032 on the dirt reference and **must read the same to five decimals with the
arm on and with it off**, because the band measures the decoded planes. If it
moves, something is applied upstream of the measurement that should not be, and
the build stops.

---

## 6. Acceptance, pre-registered (question 4)

Written before anything is built, in the order the gates run.

1. **The oracle reproduced.** Our OFF-equivalent, which is main byte for byte,
   against our ON-equivalent, at the owner's dirt views, judged by the
   **arm-internal discontinuity instrument**: the band region against its own
   surrounding dirt, per channel, stretch-proof. **Bar: the R minus G split goes
   from its measured OFF value to under one code, and the decoy region away from
   the seam does not move.** The decoy is P.1's own control and it is what says
   the change belongs to the correction and not to the scene.
   Studio reaches 0.0 by a mechanism we are not copying (section 2). A constant
   that reaches under a code is a pass. A constant that leaves most of the split
   is a **refusal**, and it is taken the way P.1's was: the instrument and the
   record are the deliverable.
2. **Interior coherence, per channel, at main's class.** Main's one gain reads
   ROUGH 0.03 percent; the rejected build 1.01. **The instrument has to be
   extended before the arm is built, and this is a finding of this memo:**
   `interior()` in `crates/spike/src/bin/colour.rs` reduces the applied field to
   LUMA through `LUMA.iter()`, so **a chroma-only stripe with no luminance lift
   reads exactly zero on the anti-acceptance metric that this whole campaign's
   photometric work is gated on.** Three numbers instead of one, with the
   planted-ripple controls per channel. The extension is a gate on the build,
   not a step inside it.
   Report the constant's pass with P.1's caveat attached: a five-term harmonic
   absorbs a constant at its zeroth term, so the pass is structural and is worth
   nothing as evidence.
3. **The green-hue pair as a gate**, `VID_20260410_185407_00_004.insv` at
   594.027 and 602.368: the owner's own sun-facing cast, OPEN and gating stage
   10, at fov 41.19 and 71.04 eight seconds apart, which asks a correction to
   hold across framing and across time. Bar: the cast is reduced at both,
   measured arm-internally on the dark ground, with no new interior roughness,
   and the two instants' corrections differ by no more than the term's own
   smoothing allows.
4. **The hard-mode view as the model's own falsifier**, clip 1 at 31.064,
   fov 142.89: glare, both crossings and the sun in one frame. This is the view
   most likely to prove a hemisphere-wide constant WRONG, because glare is local
   and a constant paints a hemisphere. Bar: no regression against the mechanism
   off, on the same instruments. Registered as a falsifier and not as a
   formality.
5. **Null byte-identity with the mechanism off.** `--bin null` at the four
   registry views against main, exactly as P.1 ran it (`down1` 7d2200ea, `down3`
   a19a9b80, `bad` f27874ed, `shimmer` 54fc67b7), plus the positive control that
   the arm on moves them.
6. **The pooled luma gain does not move**, `+0.00287` ln to five decimals, arm
   on and arm off (5.3).
7. **The standing Weber bar**, one and two pixel excess at or under the JND at
   every registry view. It is a floor and not a test: P.1 §19 showed a step laid
   across the handover reading 2.14 to 15.26 codes while both arms still passed
   it.
8. **The owner's eye, label-blind, through the hot-swap harness, and it is
   final.** Studio is the bar (owner ruling 2026-08-02).

---

## 7. The increment plan (question 5)

### 7.1 The measuring phase, which is the next run

Nothing is built until M1 answers, and M1 can refuse the whole increment.

| | what it asks | why it is first |
| --- | --- | --- |
| **M1** | the two lenses' own per-channel FAR-FIELD ratio round the ring, at the dirt reference, the green pair, hard mode and the four registry views: is there a hemisphere-scale chroma split, how big, does it reproduce within a capture | the gate. If it is inside the instrument's noise, increment 1 is refused before it is built. `--bin colour` already reads per channel; the work is pointing it at the far field the way `pool` does rather than at the seam |
| **M2** | gain or offset, fitted per channel over the ring's own 20-to-190 code range | 6.11 could not separate them on one view's flat patches, and the oracle's own correction reads offset-like. A multiplicative correction cannot fix an additive defect |
| **M3** | the weighting fork: `lit` squared, equal, Weber, three columns | `lit` squared is measured to be the wrong weight for the visible artifact (4.1) |
| **M4** | the runaway limit per channel, from the corpus, times four | `LIMIT_LN` is the achromatic number and is 28 percent of hue |
| **M5** | the ring's yield at the oracle's own view under today's six-code flatness refusal | decides whether the flat-content rule changes in increment 1 or waits (4.3) |

### 7.2 Increment 1, the smallest first build

**Constant per channel per lens, luminance-neutral by construction, two degrees
of freedom, learned by watching, applied at fusion, behind a knob and off by
default until the owner rules.**

Shape and rough size, so a build brief can be costed:

| where | what | lines |
| --- | --- | ---: |
| `crates/spike/src/bin/colour.rs` | **first**: the arm-internal discontinuity statistic as a mode this tree runs, so the oracle's own number stops being a thing one session measured once; and the interior block per channel with per-channel plants | 200 to 300 |
| `crates/render/src/band.rs` | declare `chroma0` / `chroma1` in the measure shader (no layout change); `read_photometry` accumulates two chroma means over the same 441 samples; `Cell` grows two floats and `Cell::write` appends, which is its own documented rule; `pool` fits the two chroma degrees of freedom; `Tone` grows two floats and `split()` returns a 2x3; `ALONG_AT` moves by one constant; the Rust twins beside each | 250 to 350 |
| `crates/render/src/scene.rs` | `picture()` multiplies by a `vec3` per lens | 5 |
| `crates/render/src/twin.rs` | compile the band's `tone_split` against `Tone::split`, with two planted mutations | 60 |
| the A/B arm | one row in `ab.rs`'s knob table plus `kjerag_render::ask_chroma`, which needs PR #175 merged | 30 |

Call it **600 to 900 lines** of Rust and WGSL including the doc comments these
files are written in, and expect the calendar to be mostly section 7.1.

**The A/B**: the hot-swap harness, arms as runtime configs, one trial per
photometric registry view, three arms - off, the fitted term, and the term at
double size as a sensitivity arm he is not told about - label-blind on his own
footage, one window, `--cage` for us and never the bare runner.

### 7.3 Phase 2, only on named evidence

Every row of the table in 3.3, and nothing outside it. **Per-azimuth is never
proposed again without the stage-7/8 bar**, which is now interior coherence per
channel at main's class on dark content, plus a held-out capture, plus his eye.
A local seam-centred field of the kind Studio actually applies (section 2) is
the belt's neighbourhood and is out of this line's scope.

---

## 8. Open questions, ranked

1. **Does a hemisphere-scale chroma split exist at all?** Everything downstream
   assumes it. M1 answers it in one run, and P.1 is the precedent for a stage
   step that was designed carefully and then refused on exactly this kind of
   question, at a cost of one branch and no picture.
2. **Gain or offset.** 6.11 could not separate them and chose multiplicative
   because it cannot lift a black; the oracle's own correction reads offset-like
   inside its lobes; and "the correction is multiplicative and the difference is
   additive" is the first of the three measured reasons stage 7 could not reach
   the artifact. The ring's own dynamic range can settle it now and could not
   then.
3. **The weighting.** Inheriting `lit` squared inherits a measured mistake
   (4.1).
4. **The flat-content refusal**: the defect lives on sky and flat soil, which is
   20 to 64 percent of the ring, and the band refuses to read it (4.3).
5. **Where the term lives**, and therefore whether the twin guard grows or a
   readback appears on the display path (5.2). Recommendation is to grow the
   guard.
6. **Does a hemisphere-wide constant survive hard mode**, where the cause is
   local glare and the correction is global (gate 4)?
7. **The far-field change is an owner question, not an agent's.** Studio changes
   nothing away from the seam; a constant model changes both hemispheres by half
   the split. Top of the PR body, relayed as a question, before he is asked to
   test anything.
