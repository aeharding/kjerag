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

---

# M-results: the measuring phase, run

**Date:** 2026-08-09, the run after the memo above. Everything from here down is
measurement. Nothing is built.

**What changed under this section while it was being taken, and it is not a
detail.** The owner ruled mid-run that Studio's chromatic correction is
**seam-line aware**: *"To be clear, the chromatic fixing Studio is doing is
definitely seam line aware. In other places it only makes changes close to the
seam line."* That demotes section 3's constant-per-lens model from *the* build
target to *at most the DC term of a seam-local field*. **Every measurement below
was designed against the memo above and none of them changes**: M1 stops being
"is there a hemisphere split, GO or REFUSED" and becomes "how much of the split
is a constant the field can carry as its DC, and how much is local"; M2, M3, M4
and M5 feed the local estimator's transfer function, weighting, guard and
refusal rule directly. Section M-9 states the recommendation in the new frame.

**How everything below was run.** One command per capture, CPU only, eight
frames at each of three places minutes apart in the same file, 72 azimuths round
the seam:

```sh
cargo run --release -p kjerag-spike --bin colour -- <file.insv> \
  mode=chroma from=<t> count=8 places=3 patches=72
```

Eight instants over six flights: the May file's dirt reference (488.855) and its
wide view (630.763), the April file's green pair (594.027 and 602.368), hard
mode (clip 1, 31.064), and the three registry flights
(`VID_20260501_183417_00_002` 65.666, `VID_20260714_193252_00_006` 36.303,
`VID_20260501_183417_00_003` 99.032). 1704 to 1728 azimuth-frames each, about
325 000 paired samples each.

---

## M-0. The instrument, and the two extensions the M-numbers stand on

Both extensions the memo names are built and are modes this tree runs.

### M-0.1 `interior()` per channel, with per-channel plants (memo 6.2)

The finding the memo predicted is now a printed row rather than a claim. On the
ANTI-ACCEPTANCE view (`VID_20260526_191025_00_004` 630.763, `yaw=-86.02
pitch=-17.08 fov=114.41 lock=1`), through `mode=profile`, ROUGH percent:

| planted ripple, 8 cycles round the ring | luma | R | G | B |
| --- | ---: | ---: | ---: | ---: |
| nothing at all | 0.00 | 0.00 | 0.00 | 0.00 |
| 0.5 codes of LUMA (R, G, B all +0.5) | **2.04** | 1.71 | 2.16 | 2.08 |
| 2.0 codes of LUMA | **8.15** | 6.82 | 8.64 | 8.32 |
| 0.5 codes of CHROMA (R +0.50, G -0.20, B +0.50) | **0.00** | 1.71 | 0.86 | 2.08 |
| 2.0 codes of CHROMA (R +2.00, G -0.80, B +2.00) | **0.00** | 6.82 | 3.44 | 8.32 |

The two luma rows reproduce the published 2.07 and 8.27 percent, which is what
says this is the same statistic and not a new one. **The two chroma rows are the
finding.** They carry exactly zero luminance by construction (`Plant::chroma`
puts G at `-(0.2126 + 0.0722) / 0.7152` of R and B, and the instrument prints
the luma lift so the equality is checked rather than asserted), and the luma
column reads **0.00 percent** for a stripe that a per-channel reading calls 6.8
to 8.3. A chroma-only stripe of two codes round the ring was, until today,
exactly invisible to the one anti-acceptance metric this campaign's whole
photometric line is gated on. It is not now.

Main itself, drawn: applied 0.004 codes, ROUGH 0.01 percent in luma and 0.01 in
each of R, G and B, over 88 bins. **Viewed**, not just tabulated: the amplified
difference between the band held and the band drawing
(`scratch/chromatic/interior/...-3-what-moved.png`) is a flat mid-grey field
with no structure anywhere in it, which is what an applied field of four
thousandths of a code looks like. The marked render beside it
(`...-4-marked.png`) shows the seam, the crossover and the overlap running
corner to corner across BOTH the sunset sky and the dark ploughed soil, which is
the reason M-0.2 needs a content window.

### M-0.2 The arm-internal per-channel statistic, as `mode=arm`

The oracle's own instrument, and it is a mode now rather than a thing one
session measured once. The band region is the handover's own half-width asked of
the map the render was drawn with (4.00 degrees a side today); its surround is 1
to 5 degrees past that edge; the split is the band's warmth minus the
surround's, which is a difference taken inside one picture and is therefore
**stretch-proof**; and it is reported in codes, as a share of the warmth it sits
on, and as a log ratio of ratios, which is exactly invariant to any per-channel
gain applied to the whole picture.

Positive-capable before it clears anything. On the dirt reference, `+2 codes of
R on the band alone` moves the R-G split from 2.40 to 4.40 codes, **exactly the
two codes planted**, and leaves B-G at -2.02 untouched; a 2-code green-magenta
stripe moves both by 2.79, which is the 2.796 the plant's own arithmetic asks
for. The decoy circle is read every time and does not move.

**And it settles memo 5.3's claim as a measurement.** The pooled luma gain must
not move a chroma statistic, and over five views the `band held off` and `as it
draws` rows agree to 0.01 codes and 0.0001 ln everywhere:

| view | level | R-G split, band held / drawn | B-G split, band held / drawn |
| --- | ---: | ---: | ---: |
| dirt reference, 488.855, fov 60 | 74.5 | 2.40 / 2.40 | -2.02 / -2.03 |
| May wide, 630.763, whole frame | 67.0 | 2.24 / 2.24 | -2.24 / -2.24 |
| May wide, 630.763, **soil only** | 16.6 | -0.01 / -0.01 | +0.52 / +0.52 |
| green pair a, 594.027 | 39.6 | 1.81 / 1.81 | 1.29 / 1.29 |
| green pair b, 602.368 | 40.0 | 2.27 / 2.27 | 1.92 / 1.92 |
| hard mode, 31.064 | 80.5 | -1.07 / -1.07 | -0.87 / -0.87 |

The soil-only row is the one to read on Weber grounds, and it says something
useful about which view is which: on the May wide view's own dark soil, where
the owner's complaint was measured as an ADDITIVE 6.5-code step, the hue split
is **-0.01 and +0.52 codes**, so that view's defect is luminance and not colour.
The green pair reads 1.81 / 1.29 and 2.27 / 1.92 codes on 40-code ground, which
is 4.5 and 3.4 percent in log terms, and that is the owner's reported cast.

**A caveat with a number on it.** The `rel %` column divides by the surround's
own warmth, and on the green pair that warmth is 0.18 codes, so the column
prints 1024 percent and means nothing. The stretch-proof log column is the one
to quote when the denominator is near zero.

---

## M-1. Does a hemisphere-scale chroma split exist at all? (the gate)

### M-1.1 A defect found while pointing the instrument, and it is worth naming

The memo says to point `--bin colour` "at the far field the way `pool` does".
Doing that literally reads `Cell::disparity` and cuts at
`band::NEAR_KNEE_DEG`. **That is wrong here for the reason 4.2 already gave**:
a direction whose patch never correlated is sampled at a shift of **zero**, so
reading the shift alone calls it far field and pools it with the horizon. On
the dirt reference that mistake moves 40 percent of the ring into the far-field
bin and changes the answer. Three populations are kept apart instead:
**correlated far field** (what the shipped pass actually pools), **correlated
near field**, and **never correlated** (read at zero shift, which is 4.3's
content). Every table below says which.

### M-1.2 The two chroma coordinates are arm-internal by construction

`R-G` is `ln(m1_R/m0_R) - ln(m1_G/m0_G)`: a difference of two log ratios taken
on the same patch of the same frame through the same two lenses. Everything
common to the three channels at that direction cancels **exactly** - the
scene's own level, the shading, the shutter, the auto-exposure loops, and stage
3's pooled gain itself. That is the arm-internal discipline applied to the ring
rather than to a drawn view, and it is why these numbers do not need a
reference.

### M-1.3 The noise floor, demonstrated with plants

Two nulls and three plants, every one of them a `Trial` running the same code on
the same frames with one multiplier changed. On the dirt reference:

| trial | R-G read | B-G read | what arithmetic requires |
| --- | ---: | ---: | --- |
| lens 0 against ITSELF at the found shift | **-0.00007** | **+0.00008** | 0, 0 |
| lens 0 against itself, no alignment | 0.00000 | 0.00000 | 0, 0 exactly |
| lens 1 times 1.02 in R alone | **+0.01729** | -0.02176 | +0.01729 / -0.02176 |
| lens 1 times 1.01 / 0.99 / 1.01 | **+0.01749** | **-0.00176** | +0.01749 / -0.00176 |
| lens 1 plus 4 codes in R alone | +0.03158 | -0.02271 | level-dependent, see M-2 |

The plants come back **to five decimals, in the channel they were put in and in
no other**, on all eight instants. The instrument's own bias floor is 0.00008 ln
on the dirt reference and 0.00000 to 0.00676 across the corpus. On directions
read at a shift of zero the null is zero *by arithmetic* rather than by
measurement, so there the standard error is the whole of the floor and is
reported as such.

### M-1.4 The answer, over eight instants on six flights

Pooled the shipped way (a weighted ratio of means, then logged, `lit` squared),
10 percent trimmed per reading, over the whole ring:

| instant | R-G, ln | B-G, ln | se | signal / se | floor, ln | place-to-place span, B-G |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| May dirt reference 488.855 | -0.0025 | **-0.0218** | 0.0033 | 6.6 | 0.00008 | 0.0034 |
| May wide 630.763 | -0.0072 | **-0.0172** | 0.0020 | 8.8 | 0.00001 | 0.0093 |
| April green a 594.027 | -0.0013 | **-0.0166** | 0.0028 | 4.3 | 0.0032 | 0.0266 |
| April green b 602.368 | -0.0185 | **-0.0144** | 0.0033 | 5.6 | 0.0023 | 0.0264 |
| hard mode 31.064 | -0.0003 | **-0.0196** | 0.0035 | 5.5 | 0.0000 | 0.0157 |
| registry `..._002` 65.666 | -0.0067 | **-0.0114** | 0.0030 | 2.5 | 0.0000 | 0.0067 |
| shimmer 36.303 | -0.0069 | -0.0062 | 0.0024 | 2.8 | 0.00012 | 0.0306 |
| registry `..._003` 99.032 | -0.0017 | **-0.0196** | 0.0026 | 5.5 | 0.0000 | 0.0033 |

**A hemisphere-scale chroma split exists, it is one coordinate and not two, and
it is far outside the instrument's noise.** `B-G` is **negative on all eight
instants**, between -0.006 and -0.022 ln (0.6 to 2.2 percent), at 2.5 to 8.8
standard errors from zero, and 25 to 1300 times the instrument's own bias floor
where that floor is measurable at all. Six of the eight clear 4.3 se.

`R-G` does **not** do this. It runs -0.0003 to -0.0185 with no consistent size,
and on six of eight instants its place-to-place span inside one capture is
**larger than the reading itself**. The green-magenta axis is not a
hemisphere-scale property of these cameras. The blue-amber axis is.

### M-1.5 How much of it is a CONSTANT, which is the question the steer asks

The per-direction readings averaged round the ring and fitted through the five
terms a smooth field can have. Each column is what the fit **leaves**, in ln rms
over the directions; `DC var %` is the share of the ring's variance the constant
alone accounts for.

| instant | coord | nothing | constant | +1 cycle | +2 cycles | DC value | DC var % |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| May dirt | R-G | 0.0209 | 0.0208 | 0.0157 | 0.0155 | -0.0027 | 1.7% |
| May dirt | **B-G** | 0.0300 | 0.0199 | 0.0195 | 0.0176 | **-0.0222** | **56.1%** |
| May wide | R-G | 0.0163 | 0.0147 | 0.0113 | 0.0112 | -0.0068 | 18.0% |
| May wide | **B-G** | 0.0224 | 0.0152 | 0.0117 | 0.0114 | **-0.0163** | **54.4%** |
| April green a | R-G | 0.0248 | 0.0248 | 0.0202 | 0.0197 | -0.0009 | 0.1% |
| April green a | **B-G** | 0.0241 | 0.0171 | 0.0168 | 0.0165 | **-0.0169** | **50.1%** |
| April green b | R-G | 0.0303 | 0.0230 | 0.0195 | 0.0183 | -0.0194 | 42.4% |
| April green b | B-G | 0.0257 | 0.0210 | 0.0198 | 0.0184 | -0.0146 | 33.2% |
| hard mode | R-G | 0.0238 | 0.0238 | 0.0142 | 0.0138 | +0.0010 | 0.2% |
| hard mode | B-G | 0.0295 | 0.0231 | 0.0221 | 0.0203 | -0.0180 | 38.5% |
| registry `_002` | R-G | 0.0299 | 0.0285 | 0.0219 | 0.0165 | -0.0088 | 8.8% |
| registry `_002` | B-G | 0.0248 | 0.0209 | 0.0191 | 0.0155 | -0.0132 | 29.1% |
| shimmer | R-G | 0.0181 | 0.0165 | 0.0148 | 0.0146 | -0.0074 | 16.9% |
| shimmer | B-G | 0.0145 | 0.0142 | 0.0142 | 0.0134 | -0.0031 | 4.8% |
| registry `_003` | R-G | 0.0242 | 0.0239 | 0.0191 | 0.0171 | -0.0034 | 2.1% |
| registry `_003` | **B-G** | 0.0281 | 0.0194 | 0.0187 | 0.0170 | **-0.0200** | **52.2%** |

**The constant reaches about half of the blue-amber split and no more.** On the
four strongest instants the DC accounts for 50 to 56 percent of the ring's
variance in B-G; on the weakest it accounts for 5. Adding one cycle and two
cycles buys another 2 to 20 percent, and then it stops: 0.011 to 0.020 ln of
per-direction structure survives every smooth ring model on every capture. That
residue is **local to the seam by definition** - it is what varies from
direction to direction faster than two cycles - and it is the same size as the
DC itself.

In R-G the constant reaches essentially nothing (0.1 to 18 percent on six of
eight), which is the same finding from the other side: R-G is local, all of it.

**This is the number the owner's steer predicts.** A correction that is only a
constant per lens reaches at most half of one of the two chroma coordinates and
none of the other. A seam-local field that carries a DC reaches both. The
constant is not refused; it is **demoted to a term inside the field**, which is
exactly what the steer said.

---

## M-2. Gain or offset (the thing 6.11 could not settle)

Fitted per channel over the never-correlated population, which is the one that
spans the ring's real dynamic range. `leaves` is what each candidate correction
leaves at the seam, in codes rms, applied as a symmetric split.

| instant | ch | span, codes | nothing | gain alone | offset alone | gain AND offset | fitted offset |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: |
| May dirt | R | 20-190 | 4.84 | 4.83 | 4.52 | **4.11** | -4.16 |
| May dirt | G | 15-183 | 3.71 | 3.68 | 3.59 | **2.98** | -3.43 |
| May dirt | B | 16-193 | 4.69 | 4.33 | 3.38 | **3.24** | -4.33 |
| May wide | R | 20-208 | 3.97 | 3.94 | 3.59 | **3.16** | -4.25 |
| May wide | G | 15-195 | 3.42 | 3.35 | 3.34 | **2.46** | -4.05 |
| May wide | B | 18-224 | 4.17 | 3.93 | 3.44 | **3.33** | -3.41 |
| April green a | R | 10-253 | 7.20 | 7.08 | 6.73 | **6.57** | +5.49 |
| April green a | B | 13-255 | 3.98 | 3.75 | 3.97 | **3.49** | +2.39 |
| hard mode | R | 21-230 | 8.46 | 8.35 | 7.92 | **7.60** | +7.54 |
| hard mode | G | 23-226 | 4.95 | 4.70 | 4.32 | **4.18** | +4.48 |
| shimmer | B | 13-257 | 5.27 | 5.26 | 5.00 | **4.30** | +4.76 |
| registry `_003` | B | 12-245 | 3.47 | 3.02 | 3.40 | **2.81** | +1.93 |

**The answer is: both, and the offset is the bigger half.** On 23 of the 24
channel-fits over the eight instants, `gain and offset together` leaves the
least; and `offset alone` beats `gain alone` on 19 of 24. The fitted offsets are
**2 to 7 codes** and they are not one-signed across flights: negative on the May
file, positive on April, hard mode, shimmer and `_003`.

**What that costs a multiplicative model, stated plainly.** A pure gain leaves
3.0 to 8.4 codes on the same content the offset model takes to 2.5 to 7.6. On
the May file a gain removes 0.2 to 0.9 of a 4-code step; adding the offset
removes 0.7 to 1.5. *"The correction is multiplicative and the difference is
additive"* was the first of the three measured reasons stage 7 could not reach
the artifact, and the ring's own dynamic range now says the same thing at 10 to
255 codes rather than on one view's flat patches.

The far-field cut on its own cannot settle this and it is worth saying why: on
these captures the correlated far field spans 22 to 181 codes on the May file
but **101 to 246** on registry `_002` and **65 to 231** on the green pair. It is
mostly sky. The population that spans a real black-to-white range is the one the
shipped pass reads nothing on.

---

## M-3. The weighting fork, three columns

Pre-registered rather than chosen. One estimator, three weights: `lit` squared
(shipped, and the inverse-variance weight for a log ratio), equal weight, and
Weber, which is `1 / lit` squared - the deliberate inverse, pricing a direction
by how VISIBLE a fixed error is there rather than by how many photons it has.

| instant | B-G, `lit`² | B-G, equal | B-G, Weber | R-G, `lit`² | R-G, equal | R-G, Weber |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| May dirt | -0.0218 | -0.0273 | **-0.0451** | -0.0025 | -0.0017 | -0.0032 |
| May wide | -0.0172 | -0.0135 | **-0.0078** | -0.0072 | -0.0064 | +0.0004 |
| April green a | -0.0166 | -0.0216 | **-0.0349** | -0.0013 | +0.0065 | **+0.0439** |
| April green b | -0.0144 | -0.0198 | -0.0269 | -0.0185 | -0.0244 | -0.0099 |
| hard mode | -0.0196 | -0.0246 | **-0.0475** | -0.0003 | +0.0081 | +0.0185 |
| registry `_002` | -0.0114 | -0.0175 | -0.0177 | -0.0067 | -0.0186 | -0.0359 |
| shimmer | -0.0062 | **+0.0075** | **+0.0251** | -0.0069 | -0.0167 | -0.0376 |
| registry `_003` | -0.0196 | -0.0183 | -0.0042 | -0.0017 | -0.0061 | +0.0025 |

**The fork is not decidable from the ring, and that is the result.** The three
columns disagree by a factor of two to three on five of eight instants, they
disagree by a factor of **four** on registry `_003`, and on shimmer they
disagree in **sign**. Weber does not simply amplify the shipped answer; it goes
the other way as often as it goes further.

The reason is in the split at each ring's own median level:

| instant | median | dark half `lit`² | dark half Weber | light half `lit`² | light half Weber |
| --- | ---: | ---: | ---: | ---: | ---: |
| May dirt | 60 | -0.0249 | -0.0606 | -0.0221 | -0.0232 |
| May wide | 83 | +0.0051 | -0.0086 | -0.0194 | -0.0201 |
| April green a | 98 | -0.0326 | -0.0727 | -0.0170 | -0.0206 |
| hard mode | 100 | -0.0103 | -0.0491 | -0.0219 | -0.0230 |
| registry `_002` | 122 | -0.0369 | -0.0027 | -0.0105 | -0.0117 |
| shimmer | 83 | +0.0362 | +0.0322 | -0.0105 | -0.0088 |
| registry `_003` | 117 | -0.0236 | -0.0011 | -0.0194 | -0.0192 |

(B-G throughout.) **On the light half the three weightings agree to within 0.002
to 0.004 ln everywhere.** On the dark half they disagree by up to 0.04 and they
change sign. So the whole of the fork's disagreement is the dark half's own
noise, and the fork is a question about how much to trust the dark half rather
than a question about the weight.

**What that means for the build, and it is not what the memo expected.** The
memo's complaint about `lit` squared is right about visibility and wrong about
what to do next. `lit` squared does under-weight the content the owner is
looking at. But re-weighting toward that content does not recover a better
estimate of it - it recovers **noise that is four times the size of the
signal**. The dark half of the ring, read at zero shift on flat soil, does not
support a per-direction chroma reading at this instrument's precision.

The way out is not a weight. It is **more samples per reading on the dark
directions** (a longer patch, or pooling frames before pooling directions) or a
correction whose support is local enough that it only ever has to answer where
it has evidence. That second one is the steer's own field. The fork's verdict is
therefore: **keep `lit` squared for any DC term** (it is the only column that
does not change sign anywhere, and it is the inverse-variance weight for exactly
this quantity), **and do not use a re-weighting to reach dark content**; reach
it with support and with evidence instead. The oracle's own statistic at the
owner's dirt views, which the memo names as the tie-breaker, is M-0.2 and reads
the green pair at 1.8 to 2.3 codes with `lit` squared already.

---

## M-4. The runaway guard, per channel

`LIMIT_LN` is 0.25 because the achromatic ratio was **fitted over whole
captures** and the guard is the widest fit, times four. Same derivation here,
and it matters that it is the fit and not the widest single reading: the widest
single chroma reading in this corpus is **0.61 ln**, which is a dark patch's
noise and not a camera.

| instant | widest FITTED chroma coordinate over the three weightings, ln |
| --- | ---: |
| May dirt | 0.0451 |
| May wide | 0.0172 |
| April green a | 0.0439 |
| April green b | 0.0269 |
| hard mode | **0.0475** |
| registry `_002` | 0.0359 |
| shimmer | 0.0376 |
| registry `_003` | 0.0196 |

Widest over the corpus: **0.0475 ln**, on hard mode. Times four:

> **`LIMIT_CHROMA_LN = 0.19`**, which is 21 percent of hue.

Nothing measured is clipped by it, which is what keeps it a guard rather than a
tuning knob. Under `lit` squared alone the widest is 0.0218 and the guard would
be 0.087; 0.19 admits the Weber column too rather than half-correcting a
capture whose dark half is loud. Say both numbers in the build brief and pick
the one the estimator's own weighting justifies.

---

## M-5. What the ring refuses, and to whom

Shares of the azimuth-frames tried (72 azimuths times 24 frames = 1728):

| instant | correlated far | correlated near | **never correlated** | flat (under the 6-code gate) | blind AND flat |
| --- | ---: | ---: | ---: | ---: | ---: |
| May dirt | 13.1% | 9.5% | **77.4%** | 36.3% | 27.1% |
| May wide | 7.9% | 11.7% | **80.4%** | 36.2% | 31.5% |
| April green a | 4.0% | 6.7% | **88.9%** | 44.8% | 43.1% |
| April green b | 4.2% | 10.6% | **85.2%** | 48.4% | 46.2% |
| hard mode | **0.1%** | 15.0% | **85.0%** | 56.4% | 55.0% |
| registry `_002` | 4.9% | 12.3% | **82.8%** | 43.8% | 40.7% |
| shimmer | 2.9% | 19.9% | **77.2%** | 31.0% | 29.2% |
| registry `_003` | 4.5% | 12.0% | **83.5%** | 47.5% | 46.8% |

**The shipped pass pools 0.1 to 13 percent of the ring, and on hard mode that is
one direction.** The memo quoted 20 to 64 percent of the ring as flat; measured
at 72 azimuths on the owner's own captures with the correlation gate applied as
the pass applies it, what the pass reads **nothing** on is 77 to 89 percent.
The memo's number was the contrast gate alone; this is the contrast gate plus
everything else that stops a patch correlating.

And the two populations do not disagree. On the dirt reference the far field
reads B-G -0.0244 and the never-correlated directions read -0.0226, on 950
readings against 145. Across the corpus the never-correlated pool is within
0.002 to 0.007 ln of the far-field pool on B-G wherever both have enough
readings to speak.

**So 4.3's rule change buys evidence and does not buy bias**, and that is a
measurement rather than the hope the memo recorded. It is also no longer
optional at the size the correlated far field turns out to be: an estimator that
reads only what the pass pools has one direction to work with on hard mode.

---

## M-6. Temporal: how fast does it move, and what filter class is that?

Per-frame pooled readings over 24 frames spanning three places minutes apart.

| instant | frame-to-frame rms, R-G / B-G | worst single step | whole-run span | place-to-place span, B-G |
| --- | ---: | ---: | ---: | ---: |
| May dirt | 0.0035 / 0.0016 | 0.0141 / 0.0060 | 0.0178 / 0.0070 | 0.0034 |
| May wide | 0.0032 / 0.0022 | 0.0105 / 0.0088 | 0.0121 / 0.0099 | 0.0093 |
| April green a | 0.0031 / 0.0067 | 0.0108 / 0.0244 | 0.0239 / 0.0265 | 0.0266 |
| April green b | 0.0097 / 0.0065 | 0.0409 / 0.0235 | 0.0466 / 0.0274 | 0.0264 |
| hard mode | 0.0052 / 0.0035 | 0.0191 / 0.0162 | 0.0366 / 0.0179 | 0.0157 |
| registry `_002` | 0.0018 / 0.0018 | 0.0059 / 0.0061 | 0.0116 / 0.0082 | 0.0067 |
| shimmer | 0.0072 / 0.0064 | 0.0240 / 0.0196 | 0.0288 / 0.0452 | 0.0306 |
| registry `_003` | 0.0047 / 0.0018 | 0.0153 / 0.0045 | 0.0228 / 0.0088 | 0.0033 |

**It is a constant seen through noise, and the tone gain's filter class is
right.** The per-frame reading moves by 0.0016 to 0.0067 ln rms between
consecutive frames, which is 10 to 40 percent of the signal itself, so the
per-frame reading is not usable raw. `TAU_GAIN_S` is `TAU_FAR_S` is 2 seconds,
which at 30 fps averages about sixty readings and divides that per-frame noise
by about 7.7, taking it to 0.0002 to 0.0009 ln - twenty to a hundred times under
the signal. **Nothing here asks for anything faster, and the memo's instinct
that a hue flicker is worse than a static cast is unopposed by any measurement.**

**Per-session, not per-frame, and not per-flight either.** The reading pooled
per place drifts by 0.003 to 0.031 ln over minutes inside one capture. On the
four strongest instants that drift is 15 to 54 percent of the signal; on the
weakest it exceeds it. So the term is a slowly-varying property of the session
and not a constant of the camera: a value learned at the top of a flight and
frozen would be wrong by a third of itself ten minutes later. A first-order ease
at `TAU_GAIN_S` tracks that drift with three orders of magnitude to spare and
needs no events, no states and no thresholds - which is the tone gain's filter
and not the anchor's. The one thing to take from the anchor stands: a seek
re-seeds rather than easing across.

---

## M-7. Is the per-direction structure REAL? (the question the steer makes central)

M-1.5 leaves 0.011 to 0.020 ln of per-direction structure that no smooth ring
model reaches, and a seam-local field is exactly a thing that would fit it.
**Whether fitting it is estimation or is stage 5's scalloping reborn on the
photometric axis turns on one measurement: does the same direction read the same
thing twice.** This was not in the memo above, because the memo was scoped to a
constant. It is the most important number in this run.

Three quantities and one test. `within` is the spread of one direction's
readings over consecutive frames **inside one place**, where the content is the
same and the two lenses have not moved: that is the instrument, in full.
`between` is the unweighted spread over directions of each direction's own mean,
with the ring's constant removed. `corrected` is `between` with `within` divided
out of it. And then the test noise cannot pass: the same azimuths - which are
directions in the **body** frame, so the same part of the lens pair - read at two
places minutes apart in the same file, correlated against each other.

| instant | coord | within, ln | between, ln | corrected | real % | two places, r |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| May dirt | R-G | 0.0370 | 0.0650 | 0.0636 | 98% | **+0.587** |
| May dirt | B-G | 0.0397 | 0.0828 | 0.0816 | 99% | **+0.566** |
| May wide | R-G | 0.0275 | 0.0442 | 0.0431 | 98% | +0.194 |
| May wide | B-G | 0.0282 | 0.0487 | 0.0476 | 98% | +0.301 |
| April green a | R-G | 0.0406 | 0.0849 | 0.0837 | 99% | +0.298 |
| April green a | B-G | 0.0374 | 0.0842 | 0.0832 | 99% | **-0.054** |
| April green b | R-G | 0.0301 | 0.0717 | 0.0709 | 99% | +0.336 |
| April green b | B-G | 0.0313 | 0.0870 | 0.0863 | 99% | +0.279 |
| hard mode | R-G | 0.0273 | 0.1111 | 0.1107 | 100% | +0.197 |
| hard mode | B-G | 0.0417 | 0.1101 | 0.1091 | 99% | +0.129 |
| registry `_002` | R-G | 0.0277 | 0.0740 | 0.0733 | 99% | **+0.695** |
| registry `_002` | B-G | 0.0301 | 0.0711 | 0.0703 | 99% | +0.459 |
| shimmer | R-G | 0.0381 | 0.0545 | 0.0528 | 97% | +0.295 |
| shimmer | B-G | 0.0505 | 0.0640 | 0.0614 | 96% | **-0.056** |
| registry `_003` | R-G | 0.0318 | 0.0689 | 0.0680 | 99% | **+0.593** |
| registry `_003` | B-G | 0.0356 | 0.0792 | 0.0782 | 99% | +0.536 |

Three findings, and the third is the one that decides the estimator's shape.

**One direction, one frame, cannot see this defect.** `within` is 0.027 to 0.051
ln, which is **larger than the hemisphere split itself** (0.006 to 0.022). A
single per-direction chroma reading has more noise in it than the whole signal.
Any estimator that acts on one direction's one reading is acting on noise. This
is the level floor and the trim earning their place: they are not fussiness.

**Averaged over frames, the per-direction spread is not noise.** 96 to 100
percent of `between` survives dividing `within` out of it, so at one place the
ring really does have 0.044 to 0.111 ln of azimuth-to-azimuth structure, two to
six times the DC. That is what a local field would have to fit, and it is large.

**But it is largely NOT a property of the lens pair.** The same body-frame
directions read at two places minutes apart correlate at only **r +0.13 to
+0.70**, positive on 14 of 16 rows but nowhere near 1, and on two captures the
blue-amber structure does not persist at all (April green a, r -0.054; shimmer,
r -0.056). At r = 0.5, three quarters of the per-direction shape at one moment
is gone a few minutes later.

**So a per-direction field fitted and held is the stage-8 failure, and this run
measures it rather than remembering it.** The structure is real at an instant,
mostly stale by the next one, and larger than the term it would be carried
inside. A local correction that reads a table of per-direction values estimated
over time paints yesterday's scene along each direction's whole sweep, which is
exactly the dark streaks the owner rejected.

**And it is a positive argument for the source-matching form.** A correction
whose local value is the CURRENT frame's own local disagreement, halved and
applied antisymmetrically, has no table, nothing to go stale, and cannot exceed
what it just measured. The 0.044 to 0.111 ln of local disagreement above is not
a nuisance to that form: it is its input, and it is measured to be real at the
instant the correction is applied. The thing this table forbids is the *other*
way of reaching local support, which is to fit and store one.

---

## M-8. The oracle, read for its SHAPE (owner steer, and the new pair)

The owner supplied a second on/off pair mid-run,
`~/Videos/studio_onoff/chromatic_calbration_2/{off,on}.jpg` (his spelling), and
ruled that Studio's correction is seam-line aware. Both pairs are now the oracle
set. The instrument is **`crates/spike/src/bin/oracle.rs`**, `--bin oracle`, and
it reads a pair through the same ffmpeg rgb24 pipe `--bin colour`'s `studio`
mode uses.

```sh
./target/release/oracle ~/Videos/studio_onoff/chromatic_calibration   tag=pair1 out=scratch/oracle places=11
./target/release/oracle ~/Videos/studio_onoff/chromatic_calbration_2  tag=pair2 out=scratch/oracle places=11
```

### M-8.0 A premise this run had to refuse first

**Neither pair is an equirectangular export.** 3840x2160 is 16:9 and a whole
equirect is 2:1; both are Studio **reframed wide views**, of two different
scenes on two different flights. So "one column is 360/3840 degrees" is the
scale of nothing here, and every width measured in picture pixels is measured
in a unit that changes along the seam. The instrument therefore fits the
projection to the seam itself under the one thing a seam is known to be, **a
great circle**, and scores the fit in pixels rather than in angle - an angular
score is degenerate and the first version of it duly chose a focal length of
845 000 px and reported a seam a third of a degree long.

Pair 1 fits stereographic at f = 674.4 px, 8.9 px rms over a 229.8 degree arc,
and the focal is identified (122.0 px rms at half f, 62.4 at twice). **Across
that arc the scale runs 38.6, 11.8 and 42.2 pixels per degree: it varies by
3.6 times inside one picture.** Pair 2's reframe barely stretches, 29.0 to 32.8
px/deg, 13 percent, and it is the control that makes the next section an
argument rather than an assertion.

### M-8.1 Both pairs are clean single-variable A/Bs, with their own floors

Far-field blocks more than 40 degrees from the fitted seam, bucketed by their
own brightness so the ends are never pooled. Pair 2, ON over OFF: **1.0002 to
0.9976** across four brightness buckets from 34 to 94 codes; worst single block
1.40 codes over 142 blocks. Pair 1 re-measured the same way: worst 0.82 codes
over 192 blocks. Away from the seam Studio changes nothing, on both.

JPEG noise, re-measured: pair 1 **32.4 percent** of pixels bit-identical, sd
2.01 / 1.34 / 1.93 codes, which reproduces section 2's 32 percent and 2.2 / 1.3
/ 2.0. Pair 2 is a quieter encode: 61.8 percent identical, sd 1.19 / 1.23 / 1.55.

**The instrument's own floor**, which every amplitude below is judged against,
is the DECOY: the real pair read about a great circle a quarter turn from the
seam, same machinery, where there is no handover. **0.886 codes rms on pair 1,
0.779 on pair 2.** An azimuth gets a verdict only if its peak clears four times
that.

### M-8.2 O1: the correction is ODD about the line, and cancels ON it

Three rival definitions of "the line" are traced independently and each is fitted
with its own great circle. On pair 1 the **zero crossing** fits to 8.9 px and the
**dark line** - the minimum of `|ON - OFF|` - fits to 9.8 px, and **those two
great circles are 0.09 degrees apart over the whole 229.8 degree arc**. Two
independent features of the difference, traced separately across the picture, are
the same plane through the camera to about one pixel.

| | pair 1 | pair 2 |
| --- | --- | --- |
| even energy over odd, projected profile | 0.003 to 0.023 | 0.009 to 0.107 |
| correlation with its own negated mirror | 0.918 to 0.972 | 0.875 to 0.980 |
| zero crossing offset from the fitted line | -0.15 to +0.49 deg (-3 to +7 px) | -0.45 to +0.11 deg (-14 to +3 px) |
| `\|d\|` at the line over `\|d\|` at the larger lobe | 0.043 to 0.29 | 0.024 to 0.224 |

**O1 PASSES.** The even part carries 0.3 to 4 percent of the odd part's energy on
pair 1; the change is odd about a line placed by a two-parameter global fit, to
within a few pixels; and the size of the change has a **deep minimum** at that
line rather than a maximum, everywhere on both pairs.

That is the algebraic fingerprint of an antisymmetric source-matching correction
under a crossfade, and it is the fingerprint because of what it cancels:
`(l0 + d)/2 + (l1 - d)/2` is the uncorrected average wherever the mix is 50/50,
which is on the line. **Viewed** and not just tabulated: `pair1-size.png` is the
amplified `|ON-OFF|` with the fitted circle drawn on it, and it is a broad bright
band running corner to corner with a **sharp black line down its exact centre**,
the red circle lying on that black line the whole way across; the band is
visibly wider in pixels at the ends than in the middle. `pair1-signed.png` shows
the far field as flat grey with no structure anywhere, green above the line and
pink below at the dirt end, **reversing** to orange above and blue below at the
sky end, which is section 2.3's hue turn seen rather than tabulated.

### M-8.3 O2: THE DISCRIMINATOR. It collapses, and it collapses in DEGREES

Each azimuth's odd profile normalized by its own peak, then compared on three
axes. The rms spread of the normalized curves about their own mean:

| | pair 1 (scale varies 3.6x) | pair 2 (scale varies 13%) |
| --- | ---: | ---: |
| on the **degree** axis | **0.0650** | **0.0601** |
| on the **pixel** axis | 0.2221 | 0.0664 |
| on each curve's own width, the floor of the comparison | 0.0642 | 0.0533 |
| 1/e half-width, degrees | 13.60 to 19.78, mean **16.65**, sd 11% | 12.78 to 15.15, mean **14.27**, sd 6% |
| half-max half-width, degrees | 11.85 to 17.05, mean **13.30**, sd 12% | 10.75 to 12.93, mean **11.82**, sd 6% |
| the same half-max width, **pixels** | 144.7 to 470.8, sd **44%**, factor **3.25** | 313.7 to 384.0, sd 6% |

**VERDICT: FIXED SHAPE times VARYING AMPLITUDE, and the fixed axis is the
camera's own angle.** On pair 1 the degree-axis spread (0.0650) is equal to the
floor of the comparison (0.0642) and 3.4 times better than the pixel axis
(0.2221); the pixel widths vary by 3.25 times and the angular widths by 1.44,
and the ratio between those is the reframe's own 3.6x scale variation. Pair 2 is
the control: its reframe barely stretches, so degrees and pixels agree there
(0.0601 against 0.0664) exactly as they must if the axis is what makes the
difference. **Viewed:** `pair1-profiles.png` has ten normalized curves lying on
top of each other - a near-vertical rise through zero at the line, a peak three
to seven degrees out, a long decay to zero by thirty - with one black outlier
that wanders, and the outlier is the azimuth that crosses the pilot's own body.

Two different scenes and two different exports give half-max half-widths of
**13.30 and 11.82 degrees**. The kernel is about **12 to 13 degrees of
half-width, reaching zero by 30 degrees**.

**The owner's observation is right about pixels and wrong about the sphere.**
"Doesn't seem to be a fixed distance from the seam" is exactly what a fixed
angular kernel looks like on a render whose scale changes 3.6 times along the
seam: 145 px wide at one end and 471 px at the other.

Amplitude, by contrast, varies enormously and that is where the along-seam
freedom lives: 4.88 codes on 29-code dirt (**16.9 percent**) at one end and 0.83
to 3.47 codes on 155 to 188 code sky (**0.5 to 2.1 percent**) at the other, with
the hue axis walking from +0.72/+0.29/+0.63 (R and B together against G) to
-0.75/-0.36/+0.55 (R against B). Section 2.3, re-measured on a fitted sphere.

### M-8.4 O3: half answered, and the half it cannot answer is said plainly

The symmetric-split half passes: the two lobes of the correction are the same
size, min over max **0.80 worst and 0.92 mean on pair 1**, 0.54 and 0.80 on
pair 2.

The bound half is **not settleable from a stitched output and is not claimed**.
`l1 - l0` is not in the file: both frames are already crossfaded, and because
the support is a dozen degrees wide there is no band that is both outside the
correction and still one piece of scene. The instrument prints the scene's own
slope beside every step it measures and that column is what refuses the
question: on pair 1's azimuth 294.4 the apparent step is 38 codes on a scene
slope of 2.5 codes per degree. No ratio of correction to disagreement is
reported, and none should be quoted from these files.

### M-8.5 Controls

- **NULL**, OFF against OFF through the same code path: profile rms **0.00000**,
  largest value 0.00000, both pairs.
- **DECOY**, the real pair about a circle a quarter turn away: 0.886 and 0.779
  codes rms. That is JPEG noise plus scene gradient with no handover in it, and
  it is the floor every amplitude is judged against.
- **PLANT**, a known odd lobe of 6.0 codes peak and 1.50 degrees width added to
  R in a copy of OFF, about a circle **tilted 5 degrees off the real seam** so
  the tracer has to find it rather than be told: recovered at **6.000 codes**,
  peak at 1.436 against 1.500 planted, 1/e at 3.125 against 3.188, even/odd
  energy **0.000**, and the tracer's circle 0.20 degrees from the planted one and
  5.00 from the real seam. The width rows check the decay RULE and not only the
  reading, because 2.125 and 1.925 are the planted lobe's own algebra.

---

## M-9. What the seam-local field's estimator needs, and what refuses it

The memo above pre-registered a GO/REFUSED on a constant per lens. The owner's
mid-run steer replaced that question, so this section answers the one he asked
instead: **what the seam-local estimator needs, component by component, with a
measurement behind each and the stage-8 discipline stated as the gates it has to
clear.**

### M-9.1 The recommendation

**GO, on the seam-local source-matching form, and REFUSED on both alternatives
that were live before this run.** Specifically:

| component | what the measurements say | which |
| --- | --- | --- |
| **the form** | local antisymmetric source matching, each lens pulled toward the local weighted mean before the mix | O1 (odd about the line, cancels ON it, deep minimum in `\|d\|`) plus M-7 (a stored per-direction table goes stale in minutes) |
| **the DC term** | keep one. B-G is -0.006 to -0.022 ln on all eight instants and 50 to 56 percent of the ring's variance on the strongest four. R-G carries no stable DC and should not get one | M-1.4, M-1.5 |
| **the spreading rule** | a FIXED ANGULAR kernel, about 12 to 13 degrees half-width, zero by 30, measured from the seam great circle. All the along-seam freedom goes into the AMPLITUDE. Never measure or apply a width in output pixels | M-8.3 |
| **the transfer function** | gain AND offset, and the offset is the bigger half: it wins 23 of 24 channel fits and offset-alone beats gain-alone on 19 of 24, at 2 to 7 codes, not one-signed across flights | M-2 |
| **the weighting** | `lit` squared for any pooled term. The fork does not decide, and the reason is that its whole disagreement is the dark half's own noise | M-3 |
| **the refusal rule** | 4.3's change is required, not optional. The pass pools 0.1 to 13 percent of the ring; the directions it reads nothing on agree with it to 0.002 to 0.007 ln | M-5 |
| **the temporal class** | the tone gain's: a first-order ease at `TAU_GAIN_S`, no events, no states, no thresholds, a seek re-seeds | M-6 |
| **the guard** | `LIMIT_CHROMA_LN = 0.19`, from the widest fit over the corpus times four. 0.087 if the estimator uses `lit` squared alone | M-4 |

### M-9.2 The two things this run refuses

**A constant per lens as the build target: REFUSED, and demoted.** It reaches
about half of one chroma coordinate and none of the other (M-1.5), it changes
both hemispheres where Studio changes nothing away from the seam (section 2.4,
now confirmed on a second pair at M-8.1), and the residue it cannot reach is the
same size as the part it can. It survives as **the DC term of the field**, which
is where the steer put it.

**A per-direction field fitted and stored: REFUSED, on a measurement rather than
on memory.** M-7 is the number stage 8 never had: the same body-frame directions
read minutes apart in the same file correlate at only r +0.13 to +0.70, and on
two of eight captures the blue-amber structure does not persist at all. Fitting
and holding a table paints the previous minute's scene along each direction's
whole sweep. **This is not the same refusal as "local support is too free."**
Local support estimated from the current frame is fine; local support estimated
over time and stored is what the owner rejected.

### M-9.3 Why source matching is the form the evidence points at

Four measurements converge and none of them was taken looking for this.

1. **O1's dark line.** The correction cancels exactly on the handover line and
   peaks in the flanks. `(l0 + d)/2 + (l1 - d)/2` is the uncorrected average at
   50/50, and that is the only reason a correction would have a minimum where
   the defect is largest. Even-over-odd energy 0.003 to 0.023 on pair 1.
2. **The bound comes for free.** A correction whose value is half the local
   disagreement it just measured cannot exceed what it measured. Stage 8's
   additive per-direction field had no such bound and that is how it painted
   noise. M-7 says the local disagreement is real at the instant it is read
   (96 to 100 percent of the azimuth-to-azimuth spread survives the noise
   correction), which is exactly the input this form needs and the input a
   stored table does not have.
3. **The support is the band by construction.** A correction applied to each
   lens's contribution inside the mix cannot reach outside the mix, so "zero at
   both frame corners" is structural rather than a fitted rolloff. Studio's own
   kernel is wider than our crossover - 12 to 13 degrees of half-width against
   our 4-degree half-crossover - so the widths are a design question and the
   SHAPE is settled.
4. **It is the one form that carries M-2's offset without a family change.**
   Pulling a lens toward a local mean is neither multiplicative nor additive in
   the abstract; it is whatever the local disagreement is, in codes. M-2 says
   the disagreement is 2 to 7 codes of offset plus a small gain, and a
   source-matching correction expressed in codes reproduces both without being
   told which it is.

### M-9.4 The gates the build must clear, restated for the new form

Section 6's list stands, with three changes this run forces.

- **Gate 2 grows teeth it did not have.** Per-channel interior coherence is now
  a real instrument (M-0.1) and a chroma-only stripe no longer reads zero on it.
  A seam-local field is far more able to stripe than a constant was, so the
  P.1 caveat that made gate 2 "structural and worth nothing as evidence" for a
  constant **no longer applies**: for this form the gate is live evidence and it
  is the one the owner's rejection was about. Bar: main's class, ROUGH under
  about 0.03 percent, **in all four channels**, with the per-channel plants
  beside it.
- **A new gate, from M-7: the estimator must hold no per-direction state.** If
  the build stores a per-azimuth chroma table and smooths it over time, M-7 says
  what it will paint. The check is mechanical rather than statistical: the term
  applied at a direction must be a function of the current frame's decoded
  planes at that direction and of pooled quantities, and of nothing carried
  per-direction across frames except through the DC.
- **Gate 4 gets its number.** Hard mode was "OPEN, unmeasured". It is measured
  now: B-G -0.0196 ln at 5.5 se, DC accounting for 38 percent, per-direction
  structure the largest in the corpus at 0.11 ln, cross-place agreement the
  weakest at r +0.13, **and a correlated far field of ONE direction**. Hard mode
  is where a pooled estimator has almost no evidence and a local one has all of
  it, and it stays registered as the falsifier.

### M-9.5 What is still open, and what is owed to the owner

- **The kernel's width is Studio's, not ours.** 12 to 13 degrees of half-width
  is measured on their render; nothing here says our own handover wants the
  same. Our crossover is 8 degrees wide in total. Whether the correction's
  support should match the mix or exceed it is unmeasured and is a build
  decision with an owner-visible consequence.
- **O3's bound is not measurable from a stitched output** (M-8.4). It is
  measurable on our own two lenses, and that measurement belongs in the build's
  first increment rather than in this phase.
- **The far-field question from section 8.7 is now sharper, not softer.**
  Studio changes nothing more than about 1.4 codes anywhere away from the seam,
  on both pairs, and a source-matching correction inside the band changes
  nothing outside it either - so this form does NOT carry the accepted tradeoff
  the constant model did. If a DC term is added on top of the local field, it
  does. **That is the owner question**, and it should be put to him as: the
  local field alone leaves each hemisphere's own colour untouched, and a DC term
  moves both by half the split to make them agree. Studio does the first.
- **The dark half of the ring does not support a per-direction reading at this
  instrument's precision** (M-3, M-7's `within` of 0.027 to 0.051 ln). More
  samples per reading, not a different weight. If the build's estimator reads
  the band's own 441 samples per direction it inherits this; a longer patch or
  frame pooling before direction pooling is the way out.

