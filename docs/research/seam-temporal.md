# The seam over time: what to build next, and the one thing not to

**Status:** a design memo, and since 2026-08-08 also the record behind a
shipped change: increments 1 and 2 plus the arrival staging are the app's
default behaviour, and section 9 is what they measured on the way in. The
rest is still a proposal. **Date:** 2026-08-08.
**Audience:** the owner, as the checkpoint before any of it is.
**Scope:** what to do about the seam on his May-01 downward views, after
issue #171 was refused on the clock and the four-tier Studio session gave a
reference to aim at.

**How to read the evidence.** Every number below carries its domain and one
of three labels. **Measured** means an instrument produced it and the run is
on record. **Inference** means it is a reading of measured things and could
be wrong. **Owner** means he said it. Where a number is on record only in a
session transcript with no committed artifact, it says so and is not leaned
on.

---

## 0. The answer in one page

The defect he sees is not one defect. It is three, and they want three
different fixes of very different sizes:

1. **The jump.** A 47 view px correction switching on and off 84 times in
   four seconds. This is a gate in our own code, it is diagnosed to the line,
   and the fix is small.
2. **The warp.** Our correction is a *bend* spread across an 8 degree
   corridor, so when the seam moves the corridor drags the picture with it.
   His words: we "migrate/warp the bad stitches together over the 8 deg or
   whatever, which when the stitch line moves makes this seem accentuated."
3. **The residual itself.** A steady -0.90 degrees across the seam on his
   arc, which no pose can remove and no fixed calibration wrote.

The order matters. **(1) and (2) are cheap and are about how we spend an
error. (3) is expensive and is about not having the error.** #171 spent its
whole budget on (3) and delivered it in a form that made (2) worse, at a
clock the owner refused. This memo proposes doing (1) and (2) first, and
approaching (3) only in the one form that fits the clock.

And Part 1 of this round found the one piece of (3) that a pool *can* carry.

---

## 1. The design principle: spend misregistration as ghost, never as warp

**Owner ruling, 2026-08-08, verbatim:**

> A big problem in our stitching is we "migrate"/warp the bad stitches
> together over the 8 deg or whatever, which when the stitch line moves makes
> this seem accenuated. Studio instead... it seems to "phase change"? ... the
> seam doesn't seem to "warp"/distort, it just ghosts instead.

**The mechanism translation is coordinator inference, and it is this.** There
are two places a residual disagreement can go:

- **As warp.** Ramp the correction from zero at one edge of the handover to
  the full disparity at the other. Content inside the corridor is *stretched*
  by the gradient. The stretch is geometric, it is visible as distortion, and
  when the seam sweeps across the scene the stretched region sweeps with it.
  This is what `Bend::epi` does today, and stage 9 already names the number:
  applied cell by cell it scallops, **18.5 view px of correction at one end of
  a four-degree corridor** (stage9 2, measured).
- **As ghost.** Displace whole content into alignment, and let whatever is
  left over show up as *superposition* in the crossfade: the same edge twice,
  faintly, dissolving. No geometry is distorted. The doubling is bounded by
  the residual and it does not sweep.

The principle: **corrections displace, residuals ghost.** A correction that
cannot be right everywhere should leave a double image, never a bent one.

**Three measured things support it, and none of them was collected to.**

1. **The 8 degree handover won his blind A/B** (`projection.rs:104-137`,
   owner label-blind 2026-08-05, verbatim "2 is way better. Def not perfect
   but way better"). A wider handover is a *gentler* shear and a *wider*
   doubling. He chose more ghost and less warp before anyone had a name for
   it. Inference: that vote was already this principle.
2. **#171's displacement form was measured steadier than `main` at every
   band** (stage9 12.5, measured: band state rms 0.0444 to 0.0349; frame pairs
   stepping over a view pixel at +60 px, **21 of 87 to 0 of 87**). Its
   application form - a displacement of lens 1's whole picture across the
   seam, one displacement and not a ramp (stage9 10.2) - was never the thing
   that failed. The clock was.
3. **Studio has no depth model anywhere** (RE, measured negative,
   well-supported) and its own dynamic tier ghosts rather than warps at near
   content. Owner on that tier: some ghosting on his outer leg outline, the
   building pretty stable. *That is his observation and not his acceptance;
   the acceptance question goes to him at the checkpoint below.*

**What this principle retires.** The corridor-local bend as an *application
form* for a correction we believe in. It does not retire the corridor - the
crossfade is where the ghost lives, and section C3 is about its shape.

**The honest cost, stated up front.** At near content a single displacement
cannot be right for two depths at once, so the residual grows and the ghost
with it. Under this principle the near field looks like **bounded doubling**
rather than bounded distortion, and the bound is the disparity spread across
the corridor. It is not free and it is not obviously better to every eye. It
is what Studio's block-match tier looks like on his leg, and it is a question
for the A/B, not a claim.

---

## 2. What is measured, and where each number's domain ends

### 2.1 The defect on his clip

- **The residual.** `--bin crossing`, three downward views on
  `VID_20260501_183417_00_002.insv` at `fov=20 lock=1 bins=180 seam=pool`:
  medians **-46.39, -46.41 and -48.00 view px** over 8 to 9 accepted sites
  each, per-site -45.48 to -48.79. At 51.2 px/deg that is **-0.906 degrees**.
  Measured. **Domain: the unbent projection.** The band's per-frame bend is a
  second layer and is not in those numbers (`crossing.rs:47-53`).
- **The instrument's own controls, CORRECTED 2026-08-08.** The null reads
  exactly 0.0000 at every site, ncc 1.00000. This memo said, until today, that
  *"the yaw plant file for these runs is byte-identical to the unplanted run -
  a plant that moves nothing is not a positive control"*, and asked for a
  re-run before anyone quoted -0.906. **The re-run says the control was alive
  the whole time and the memo was reading the wrong artifact.** `--bin
  crossing` writes the CSV from the base readings and prints the plant's
  readout on stdout (`plant_row`, and `report`'s `plant read [...] against
  [...] predicted` line), so a plant CSV identical to an unplanted one is what
  the instrument is built to produce and says nothing at all. Re-run at all
  three downward views with `plant=yaw:0.10`, the medians read against
  prediction: **-0.0360 against -0.0391 deg, -0.0351 against -0.0391, -0.0270
  against -0.0310**, error medians **+0.0031, +0.0008 and +0.0002 deg** over 9,
  9 and 8 sites. The plant moves the reading and it moves it where the map
  says. **-0.906 degrees is positive-controlled and it stands**, and the base
  medians re-read exactly: -46.39, -46.41, -48.00 view px.
- **The structure.** Across-seam DC spans 0.666 deg over six flights against
  an along-seam floor of 0.119 (reference-views, measured). May-01 carries the
  second *smallest* DC of the six and the three clips he calls clean carry the
  three largest. What May-01 uniquely carries is **across rms 4.3x its own
  along rms (0.4214 against 0.0979)**. The defect is per-azimuth, not a
  constant.
- **No rigid pose can make it a constant.** The whole five-knob family can
  shift the ring by **0.04 degrees** of constant at the runaway bound
  (`--bin downweight`, measured); the leftover carries about half a degree of
  one. A solved pose (pool with yaw -0.283, pitch +0.872) does zero his three
  downward views (-46.39 to +0.19, -46.41 to +0.05, -48.00 to +0.08,
  measured) - and it moves individual sites elsewhere by up to **+11.5 src px
  the wrong way** (BAD arc +65.02: +7.859 to +19.358; GOOD arc -152.98:
  -7.402 to -13.314). That is the redistribution, measured per site.
- **v6 is eliminated as the fix.** `offset_v6` is byte identical on all six
  X4 Air captures, one md5 (measured), so it can only carry an error that is
  the same on every flight, and the across-seam DC is not. It stays the
  ground-truth oracle for any learn-from-footage estimator. *The claim that
  the ONE X2 has no v6 at all is not written down anywhere in the repo; treat
  it as unsourced until someone checks.*

### 2.2 The jump, which is ours and is diagnosed to the line

The band's state is already smoothed: an exponential filter per direction,
`TAU_FAR_S = 2.0 s` and `TAU_NEAR_S = 0.10 s`, blended by disparity across
`NEAR_KNEE_DEG = 0.19` (`band.rs:180-195`, measured in source). On his arc the
disparity is 0.906 deg, so `|d|/0.19 = 4.77` clamps to 1 and **tau is 0.10 s
every frame there** (`band.rs:1284-1286`).

But the smoothed state is not what reaches the picture. On the way out it is
multiplied by an *instantaneous* trust:

```
let strength = clamp(mix2(wa, wb, mix) / KEEP, 0.0, 1.0);   // band.rs:1756
let trust    = (cell.off_conf / KEEP).clamp(0.0, 1.0);      // band.rs:439
```

with `KEEP = 0.65` (`band.rs:167`). **The state holds. The gate on the way out
does not.** Measured on his arc (`--bin band` trace, 120 frames = 4.0 s at 30
fps, 9 of 128 directions in the box, azimuths 101.2 to 123.8 deg,
`scratch/band-v1-trace.log` on `research/v6-player`):

- **84 frame-to-frame steps larger than 10 view px in 4 seconds.**
- **Worst single-frame step 46.74 view px.**
- The applied value swings between **0.00 and -47.61 view px**, while the held
  disparity underneath it sits steady at -0.9120 to -0.9138 deg.
- Confidence on those frames runs **0.657 to 0.895**, straddling KEEP = 0.65.

That is the whole mechanism. Content flickers the correlation across 0.65 and
a 47 px correction is switched on and off underneath a state that never moved.

**Also on his arc: we are nearly blind there.** `scratch/refusals.log`:
`arc: 93 to 125 deg is cells 34 to 44, of which 1 of 11 are read`, with 22
no-patch and 234 unlike moments and **the near gate firing zero times**.
Whole file: 71 of 128 directions read. Measured.

**Incidental defect found while reading, worth an issue on its own:**
`crates/spike/src/bin/band.rs:831` has `if past > 1.5 { continue; }`, a stale
1.5 degree half-corridor while the shipped handover is 8 degrees (half-width
4.0), so the instrument's `covering()` silently omits most of the handover.
Present on `main` at the same line.

### 2.3 What #171 established, and what refused it

- **The mechanism works.** Per-session per-azimuth across-seam displacement,
  delivered: BAD crossing **19.92 to 0.83 view px**, nine crossings on two
  cameras, eight of nine improved, steadier at every band (stage9 13.6, 13.8,
  measured).
- **The clock refused it.** Owner, verbatim: *"the performance of this
  approach is unworkable, we need orders of magnitude better - insta360 studio
  launches perfect seam with <2s of loading time."* The cost was **27.3 s to
  read the field and 135.1 s to walk it in** (stage9 13.11, measured). His own
  playback log contradicted the frame-rate worry in the branch's favour, so
  **the clock is the constraint and dropped frames are not**.
- **Coverage.** The shipping harvest plan read 71 of 128 directions and left
  **57 dark** on dusk content; his arc was identity (measured).
- **It was never eye-validated.** "No eye has seen any of it" (stage9 12.7).
- **The walk-in guard survives, and it is the only thing measured to tell a
  right field from a wrong one after the fact** (stage9 12.3, 13.3). Planted:
  gain 1 takes **100 percent**; gain -1, same size wrong way, takes
  **nothing**; gain 2, twice too large, takes **50 percent, which is the true
  field to the digit**. Measured.
- **The rail.** A term whose *residual* leaves the band's epi search window
  blinds the band. The window is `FAR_DEG..NEAR_DEG = -1.2..+2.6 deg`
  (`band.rs:114-115`); `EPI_LIMIT_RAD` is 2.8 deg. And stage9 12.3 is emphatic
  that the rail is not the guard: *"The safety question is not `|T|` and it
  never was: it is `|T - truth|`, and nothing knows `truth` before the band
  has measured through the term."*
- **The correction to the harvest premise, and it is the opening this memo
  uses.** The 27.3 s was reading across the whole file up front. **The band
  already measures all 128 directions every frame during normal playback.**
  Live accumulation needs no harvest.

### 2.4 Studio, and what we actually know about it

Measured by static analysis of `studio_worker.dll`, corroborated where noted:

- **Nothing is measured at open.** Stored calibration; dense flow from frame
  one starting at a **zero** field; full DIS pass. The owner's own project
  files record `optical_flow_stitching="1"`, `ai_stitch="0"`, so the tier we
  compare against is the **classical** one.
- **`frame_interval_of_flow_calc_ = 2`**, with pure hold between recomputes
  (identical baked UV maps). The symbol and the value are measured; **whether
  it means one frame in two or one in three is inference and the RE states it
  both ways.** At either reading the hold is about 70 to 100 ms.
- **Untrusted belt rows fail upward**: a 98/2 blend toward the coarse pyramid
  estimate, never a snap back to calibration. Measured.
- **No depth model anywhere.** cudastereo ships and is unreferenced. Measured
  negative.
- **Blend curve.** The RE reports a steep **quartic** concentrating the 10-90%
  crossfade into about **27% of the band**, and a nadir-hemisphere collapse to
  a hard **5 degree** floor. *The specific figures "lambda 5.2" and "26.6%"
  are transcript-only: they are not in the repo and not in the committed RE
  notes. Cite the shape, not those two digits.*
- **Trailer record 2 is a camera-authored stitched 1280x640 equirect of frame
  0** (4 seeks, ~1.2 MB). Measured, with decoded artifacts. An instant
  first-paint asset, and a per-clip acceptance reference.
- **Owner's fusion-angle experiment, 2026-08-08, and it is the discriminating
  observation.** The Stitch Fusion Angle slider maps to literal degrees with a
  roughly 1 degree floor: at 0 he gets "basically a shear line with very small
  transition, like 1 deg". This **refutes** the RE's round-2 inference that
  `blend_Angle=0` means auto in the enabled path. Their manual range is
  therefore about 1 to 7 real degrees, sitting just below our shipped 8.
  *Corollary, inference: sub-5-degree custom widths coexisting with the nadir
  5 degree clamp implies the bottom-optimize flag is off on that path.*
- **And the second half of that experiment retired a whole model.** At a
  narrow fusion angle the seam **"moves around smoothly"**. So the underlying
  Studio correction moves *continuously*; his earlier reading of "discrete
  held states with fades" was a **rendering-width phenomenon**, not a temporal
  mechanism (synthesis: coordinator inference). At a wide blend the same
  continuous change renders as ghosts fading between apparent positions. This
  also retires the stitching-off discreteness as needing an explanation.
  **Consequence: do not lean on cadence as the explanation of Studio's calm.**
  Their 3-frame hold is real and is plausibly below perceptual threshold.

**A record-keeping caveat the memo has to carry.** The four-tier Studio
session is on record only as a coordinator paraphrase; no commit, issue or doc
holds his words. The phrases "held states + fades" and "this seems to make
their shimmer less than ours" do not appear verbatim anywhere. Where this memo
quotes him verbatim it is from the repo or from this week's relayed messages;
the four-tier rankings are **second-hand** and are used only as direction, not
as a number.

### 2.5 Part 1 of this round: the sinusoid is poolable

New measurement, this branch, `--bin constant` extended to print phase. Full
table and controls in `reference-views.md`. The headline:

**On five X4 flights the across-seam one-cycle term is one vector.** chi2/dof
**1.11** against an along-axis floor of **2.00**; pooled vector **0.2477 deg
at 110.5 deg**; applying it leaves at most **0.1756 deg (2.87 src px)** on the
worst flight and buys 0.078 to 0.232 deg on every one of the five. Jul-25 is
excluded as the corpus's thinnest arc (20 sites over 195 degrees, solid
undercast, already excluded by stage9 10.3); with it in, the six read 1.99
against 2.77, still poolable by the floor but no longer agreement inside bars.

**The control that makes it worth something.** Recovery is exact by linearity
and therefore proves only that a plant arrives. The control that counts plants
a **per-session wander of known size** by turning each capture's plant 72
degrees: chi2/dof runs **1.11 unplanted, 1.57 at 0.05 deg, 2.35 at 0.10, 3.44
at 0.15, 4.84 at 0.20**, crossing the floor between 0.05 and 0.10. **A
per-session one-cycle wander of about a tenth of a degree or more would have
been found; the data sits below the smallest plant tried.**

**What it does not say.** Not that the term is a calibration error. Not that
the two-cycle order is poolable - it reads per-session (3.65 against 2.09) and
the five-term fit is at the edge of what these arcs support, so nothing should
be built on it. And not that pooling fixes his clip: 0.2477 deg is 4.0 src px
of a -0.906 deg (-14.8 src px) defect.

---

## 3. The candidates

### C1. The band's temporal behaviour: hold, and fail toward the held state

**What it fixes.** The jump (2.2), by name and by line. Nothing else.

**The core change, and it is one idea.** The applied correction is currently
the smoothed state times an *instantaneous* trust. Make the trust part of the
state: smooth it with the same `ease`/tau machinery the disparity already
uses, and on confidence loss let the applied value **decay toward the held
state, never snap to zero.**

**Interaction with TAU_NEAR and KEEP, specified.**
- `KEEP = 0.65` stays as the gate on whether a reading *enters* the state. It
  stops being the gate on how much of the state *leaves* it. Those are two
  jobs and one constant is doing both today.
- The trust's own time constant should be `TAU_FAR_S`-like (seconds), not
  `TAU_NEAR_S`. Rationale: `TAU_NEAR_S = 0.10 s` exists because *the wing
  moves* and a near reading must track it. Confidence flickering is not the
  wing moving; it is the correlator losing the scene. Those want opposite
  responses, and today they share a knob. **This is the substantive design
  claim in C1 and it is falsifiable: if a slow trust makes the wing smear, the
  A/B will show it.**
- `NEAR_KNEE_DEG = 0.19` is untouched. On his arc it will still pin tau at
  0.10 s for the disparity, which is correct: the disparity there really is
  near-field-sized even though the content is far.

**Two cadence policies, and the memo weighs both rather than picking on
taste.**

*(a) Fixed-interval recompute and hold.* Measure every Nth frame, hold
between, fade across the change. This is Studio's literal measured cadence
(interval 2). **Against it:** section 2.4's correction says their calm is
probably not cadence, and a fixed interval throws away evidence we already
have - our band measures all 128 directions every frame, and Studio's interval
exists because a full DIS pass is expensive. Ours is not.

*(b) Deadband with committed transitions.* Hold the applied state while the
measured state stays inside a deadband; commit a faded transition only when it
departs beyond a threshold **and** confidence is solid. **For it:** it spends
our asymmetry over Studio - continuous evidence used to decide *when* to move
rather than to move constantly - and the worst-case error while parked is
bounded by the deadband, which is a stated parameter rather than an emergent
property. **Against it:** it is more machinery, and it has a failure mode
(chatter at the threshold) that (a) does not.

**Its evidential basis, corrected.** Policy (b) came from the owner's
hypothesis that Studio holds a stitch line as long as it can. Section 2.4
retired that reading of Studio. **(b) stays in the memo on its own merits -
bounded parked error, clean planted tests - and not as "what Studio does".**

**What C1 cannot fix.** The -0.906 deg residual. The warp. Anything about
first-open correctness. It makes a wrong correction *steady*, which is a real
improvement to a shimmer complaint and no improvement at all to a
misregistration complaint.

**Clock and frame rate.** Free at open: nothing new is measured. Per frame it
is one extra smoothed scalar per direction in a state that already carries
several; against the shipped pass's **8.44 ms** (stage9 13.9, measured) this
is not resolvable. **Envelope: no change to either bar.**

**Line counts.** Increment 1a, smoothing the trust: **40 to 60 lines** across
`band.rs`'s Rust path, its WGSL twin and two tests. Increment 1b, the
deadband: **150 to 200 lines** including its state, its transition fade and
its planted tests.

**Risk.** Low for 1a: the change is inside one multiply and the null is
byte-identity on a file whose confidence never crosses KEEP. Medium for 1b:
new state machine, new failure mode.

**Acceptance.** Planted tests first: a planted confidence dropout must produce
a monotone decay and no step; for 1b, a planted step must produce **exactly
one** committed transition and a planted drift below the deadband must produce
**none**. Then the delivered-path instruments against `main` (`--bin step`,
`--bin shear`, band-live, `seam=pool`) plus the null byte-identity. Then his
eyes, per section 5.

### C2. Per-session per-azimuth across-seam displacement, accumulated live

**What it fixes.** The residual (2.1), which is the only candidate here that
does. #171 measured this mechanism taking BAD from 19.92 to 0.83 view px.

**Its application form is settled** by section 1 and by #171's own measurement:
a **displacement** of lens 1's whole picture across the seam, per direction,
read through by the band. Not a corridor bend. That half is not an open
question any more.

**What is open is the estimation, the persistence and the arrival**, and the
clock lives entirely there.

- **Estimation.** Accumulate from the band's own 128-direction per-frame
  readings, under the three gates #171 built (far gate on the excursion, the
  trimmed middle, the five-term shape gate). **No file harvest.** This is the
  whole difference from #171: its 27.3 s was reading across the file up front,
  and normal playback already produces the same readings for free.
- **Persistence.** Store the converged field per file, or per session key, so
  a reopen applies it at frame zero.
- **Arrival.** Walk it in under the surviving staged guard, in steps small
  enough that a wrong field's residual stays inside the band's window, with
  the band re-measuring at each step and the walk aborting when evidence falls
  or the residual grows. Measured behaviour: right field 100%, wrong-sign 0%,
  2x-too-large 50%.

**First-open behaviour, stated honestly against the 2 second bar.**

| when | what is correct |
| --- | --- |
| t = 0, first ever open of a file | calibration plus the pooled pose, which is what `main` draws today, plus C4's pooled sinusoid if it ships. **The per-session field is absent.** |
| t = 0, any later open of the same file | the persisted field, applied whole at frame zero. **Correct immediately.** |
| first open, during playback | the field accumulates and walks in gently under C1's cadence |

**This does not meet the 2 second bar on a first open and the memo says so
plainly.** Nothing that learns from footage can. What it does is make the bar
irrelevant on every open after the first, and make the first open's arrival
*calm* rather than a two-minute spectacle. Whether that is acceptable is an
owner decision and it belongs at the checkpoint, phrased as: *"the first time
you open a clip the seam improves over the first N seconds of playback; every
time after that it is right when the frame appears."* **N is unmeasured** and
is the first thing to measure if C2 is authorized.

**What C2 cannot fix.** Anything on a clip whose arc the band cannot read - on
May-01 his own arc read 1 of 11 cells (2.2). **This is C2's largest open risk
and it is specific to the clip that motivated the work.** A field that is
identity where he is looking is #171's coverage failure again. Before any C2
build, the first measurement to run is whether live accumulation over minutes
of playback closes the 1-of-11 that a 6x4 harvest could not.

**Clock and frame rate.** Open: zero, nothing is measured at open. Playback:
the readings already exist, so the added cost is the accumulator and the
walk's periodic re-fit. #171 measured the walking build at **28.95 fps flat
out against main's 29.89, and 29.53 resting** (stage9 13.9), and the owner's
own log showed 0 to 6 dropped frames per 5 s window. **Envelope: frame rate is
already measured to hold; the clock is met by persistence, not by speed.**

**Line counts.** **250 to 400 lines**, of which most is adaptation rather than
invention: the gates, the walk and the composed-term identity rule all exist
on `feat/per-session-epi`. The new code is the live accumulator and the store.

**Risk.** High, and it is the coverage risk above, not the mechanism.
Secondary: the feedback loop (the field changes what the band reads, which
changes the field) was never measured stable; the walk's abort criterion is
the only thing standing in front of it.

**Acceptance.** The nine-crossing registry battery, delivered path, against
`main`, with stage9 10.10's rule binding: **improve both May crossings without
trading one for the other.** Plus the null byte-identity before a field lands,
plus the planted walk (right/wrong-sign/2x). Then his eyes, per section 5.

### C3. The blend curve: keep the 8 degrees, spend less of it visibly

**What it fixes.** How the residual *reads*. Under section 1's principle this
is the candidate that shapes the ghost, and section 2.4's correction promotes
it: if Studio's calm is a rendering-width effect rather than a temporal one,
**this is the primary shimmer lever and C1 is the jump lever.**

**The current shape, and it is worse than assumed.** `crossover()` is a
**linear** ramp:

```
fn crossover(apart: f32, reach: f32, band: f32) -> f32 {
    (0.5 + apart / (2.0 * reach * band)).clamp(0.0, 1.0)
}                                        // projection.rs:1621
```

so the 10-90% crossfade is **80% of the support**: at `CROSSOVER_DEG = 8.0`
(`projection.rs:139`) that is about **6.4 degrees of visibly mixing picture**.
Studio's manual range is about 1 to 7 real degrees (owner's slider
experiment), and the RE reads their curve as a quartic putting 10-90% into
about 27% of the band.

**The change.** Keep the 8 degree support - it won his blind A/B and it is
what the X4 Air's 14.68 degree lens overlap affords (9.48 available, measured)
- and replace the linear ramp with a steep symmetric power curve so the 10-90%
lands at **3 to 4 degrees**. Their shape, our width.

**Why the support must stay 8.** The support is also the per-file clamp's
subject: the ONE X2's floor is **3.99 degrees with nothing to spare**
(`band.rs:228-240`, measured; 4.18 once the curve shipped, 9.6). A design that
narrows the *support* rather than the *transition* would have no room on the X2
at all.

**What C3 cannot fix.** Nothing about registration. It changes how a given
error looks, not how big it is. It could plausibly make a *large* residual
look worse (a crisper line rather than a soft double), which is exactly why it
is an A/B and not a numbers question.

**Clock and frame rate.** One extra pow or two multiplies per pixel in a pass
that already does two full lens projections. **Envelope: no change to either
bar.** Zero at open.

**Line counts.** **20 to 40 lines**: the function, its WGSL twin, and the
tests that assert the 10-90% width against the support.

**Risk.** Low and fully reversible; the exponent is one constant.

**Acceptance.** A measured 10-90% width test (the existing
`the_crossover_is_the_width_it_says_it_is` has the shape for it), the null at
exponent 1 being byte-identity with `main`, and then straight to his eyes.
**C3 is the cheapest thing in this memo and it is aimed at the percept he
described. It should go first.**

### C4. The pooled sinusoid, which Part 1 just licensed

**What it fixes.** A pose-shaped slice of the residual, on every X4 file, at
frame zero, for free.

**What it is.** Part 1 measured the across-seam one-cycle term as one vector
on five of six flights: **0.2477 deg at 110.5 deg**, chi2/dof 1.11 against a
floor of 2.00, detection floor about 0.10 deg. That is a pooled, per-camera,
zero-cost correction of exactly the kind stage9 13.11 named as the only shape
that fits the clock.

**Its honest size.** 0.2477 deg is **4.0 src px**. His defect is 14.8. **C4 is
about a quarter of it and it is not a fix for his clip.** What it is worth is
that it costs nothing, it applies at t=0, and it reduces what C2 has to walk
in later - which shortens C2's first-open arrival, the one number C2 cannot
meet.

**Where it goes.** Two options and the memo does not pick between them without
a measurement: (i) fold it into the pooled `SeamFit` as a pose refinement, or
(ii) carry it as a one-cycle across-seam displacement term. **(ii) is more
honest** - `--bin downweight` measured that no pose knob produces a constant
and only some produce one cycle, so folding a fitted sinusoid into pose knobs
may not be expressible. Measure the expressibility before choosing.

**Clock and frame rate.** Zero and zero. It is a constant.

**Line counts.** **30 to 60 lines** for form (ii), reusing the composed-term
machinery.

**Risk.** Low-medium. The risk is stage9 10.10's trade rule: a pooled term
that helps five flights on average can still hurt one crossing. It must clear
the same nine-crossing battery, and **it must not be shipped on the pool
measurement alone** - that is precisely the mistake stage9 9 documents.

**Acceptance.** Nine-crossing delivered battery, the trade rule binding, then
folded into whichever A/B build is next.

---

## 4. What this memo does not propose, and why

- **Optical flow / belt tracking.** Three reasons, in order of weight.
  (i) **The owner watched it fail in Studio's own product**, at the body /
  building depth boundary, on the AI tier; the classical tier warps as the
  camera bounces. Their flow has a decade of engineering in it and it still
  breaks where our footage lives. (ii) **The X2 constraint**: any fix must
  work from footage alone on a camera with no per-clip refinement to lean on,
  and a flow tier is the most content-dependent thing we could build. (iii) It
  is the largest possible increment and it cannot end at an owner checkpoint
  in under weeks. **Not refused forever; refused as the next thing.**
- **A depth model.** Studio has none (measured negative). We would be the only
  ones, on a battery that cannot currently distinguish which of two poses is
  better at his views.
- **A static pooled per-azimuth across-seam table.** Refused with a number:
  per-azimuth spread across flights **median 0.597 deg, worst 1.531**, against
  a pooled median's own rms of 0.229 (stage9 10.12, measured). A mean over a
  population two and a half times more spread than the mean's own size
  reconstructs no member of it. **C4 is not this**: it is one vector at one
  harmonic order, tested for poolability with a stated detection floor, not a
  128-entry table averaged.
- **Anything relying on the two-cycle term** (per-session, thin fits).
- **Anything unfalsifiable.** Every increment below has a planted control that
  can fail and a null that must be byte-identical.

---

## 5. Acceptance, and the change to the ladder

**The stills stage is removed for this work.** Owner, verbatim:

> No stills first please, we're at the point where live video is the only/best
> discriminator. I want a/b testing w the player pointed at the downward
> problematic seam.

Supporting citation from our own record: stage 9's charter already found that
paused frames are blind to the dominant percept, and the shimmer is a motion
percept by construction.

**So the ladder is now two rungs, not three.**

1. **Instruments gate first, and they still gate.** Delivered-path numbers
   against `main` (the binding rule of stage9 9.4, both halves: difference
   *and* quality). Null byte-identity. Planted controls that can fail. Nothing
   reaches his eyes without these.
2. **Then a playback A/B in the player.** A research build, A and B arms on
   the `~/kjerag-ab` harness pattern: one command per arm, zero setup,
   blinding lives in the script's authorship, he answers "1", "2" or "same"
   per view. **Pointed first at his three banked downward views on May-01**,
   then the four-tier Studio session views, the shimmer anchor, and the
   GOOD/BAD crossings.

Then, and only then, productionizing. Every increment below ends at rung 2.

---

## 6. The sequence

Smallest first, each ending at an owner checkpoint. Line counts are estimates
and are stated so he can refuse one on size alone.

**Increment 1: C3, the blend curve. ~20-40 lines.**
The cheapest change in this memo, aimed directly at the percept he described
tonight, fully reversible, no clock cost. **Deliverable: an A/B-ready build at
his three downward views**, exponent 1 (identical to `main`) against a steep
curve at the same 8 degree support. This is also the increment that tests
section 1's principle at zero risk: if concentrating the transition reads
worse to him, the ghost-over-warp principle is wrong about his eye and
everything after it should be re-thought.

**Increment 2: C1a, smoothing the trust. ~40-60 lines.**
Kills the 46.7 px jump. Diagnosed to the line, small, with a byte-identical
null. **Deliverable: an A/B build at the same views**, plus the `--bin band`
trace re-run to show the 84 steps gone.

**Checkpoint. Both of the above are about how we spend an error we still
have.** If they are enough for his eye at the downward views, the expensive
work does not need to happen. That is the honest reason to do them first.

**Increment 3: the C2 coverage measurement, no build. ~60 lines of
instrument.**
Does live accumulation over minutes of playback close the 1-of-11 on his arc
that a 6x4 harvest could not? **If the answer is no, C2 is dead and the memo
should be rewritten, not extended.** This is the cheapest possible test of
C2's largest risk and it must run before any C2 code.

**Increment 4: C4, the pooled sinusoid. ~30-60 lines.**
Only after increment 3, because if C2 is dead C4's value changes (it stops
being a head start and becomes the whole of what a pool can do). Nine-crossing
battery with the trade rule binding.

**Increment 5: C2, if increment 3 licensed it. ~250-400 lines.**
Live accumulation, persistence, staged walk-in. Measure N (the first-open
arrival time) before showing it to him, and put the first-open behaviour to
him as an explicit accepted-tradeoff question at the top of the PR.

**Increment 6: C1b, the deadband, only if increments 1 and 2 leave residual
shimmer he can name. ~150-200 lines.**
It is the largest machinery for the smallest named defect, and it should be
earned by a complaint that survives everything above it.

---

## 7. What would change this plan

- **His A/B on increment 1 going the wrong way** retires section 1's principle
  and most of this memo with it.
- **Increment 3 answering "no"** kills C2 and leaves C4 as the only thing
  addressing the residual, which is a quarter of it. That is a real possible
  outcome and the memo does not hide it.
- ~~**The `--bin crossing` plant control being genuinely broken** (2.1) would
  put a question mark on the -0.906 degree number that motivates all of this.
  Re-running it is cheap and should happen regardless.~~ **Answered
  2026-08-08: the control was never broken.** See the correction in 2.1.
- **A committed record of the four-tier Studio session in his own words**
  would let its rankings be cited as evidence instead of as direction.

---

## 8. What was built, and what it measured (2026-08-08)

Increments 1 and 2 are built behind their own research toggles, increment 3 is
measured, and all of it is staged for the owner's playback A/B at
`~/kjerag-ab/temporal-ab.sh` (three arms, six views, blind). Nothing is merged
and nothing is on by default.

**The corrections this round made to the memo above.** Two, and both matter.
The plant control was never broken (2.1, corrected in place). And section C3's
"6.4 degrees" is the crossfade measured on the **share** axis rather than in
degrees of picture: the weights are cosines of two lens axes, so the delivered
10-90 percent of an 8 degree handover is **4.89 degrees** and not 6.4
(`the_along_seam_correction_hands_over_across_the_whole_crossover`, on record
since 2026-08-05 at 0.61 of the width). An exponent chosen on the share axis to
hit "3 to 4 degrees" would have been 2.35 and would have delivered 2.27.

### 8.1 C3, `KJERAG_BLEND_CURVE=steep`

`crossover()` keeps its support and re-spends the share on
`s^n / (s^n + (1-s)^n)` at `n = 1.5`. Delivered 10-90 percent, swept on the
calibration fixture over 24 azimuths at the shipped 8 degree support:

| exponent | 1 | 1.3 | **1.5** | 1.8 | 2 | 2.35 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| delivered, deg | 4.89 | 3.93 | **3.46** | 2.93 | 2.65 | 2.27 |

**A defect found on the way, and it had a failing case.** The fold guard is
`gradient * disparity / band <= 1`, held at 0.9, and it was written where the
gradient was 1 because a linear ramp's is. This curve's peak gradient is its
exponent. At the ONE X2's support against a search that reads out to 2.6
degrees the undivided shear is **over 1** - the steep arm folds a real camera
(`the_blend_curve_cannot_fold_the_narrowest_camera`).

**The division as written here was half of one**, and 9.6 has the rest: it went
into `band::carried` and not into the four other functions that solve the same
inequality, which is a defect this memo's own "one inequality, read two ways"
would have caught. It ships as one derived constant, `band::SPEND`, which all
five read. Section 8.1's delivered figure is also a prediction rather than a
measurement; 9.5 has that.

### 8.2 C1a, `KJERAG_TRUST=smooth`

The gate on the way out becomes `Cell::trust`, filtered at `TAU_TRUST_S = 2 s`.
`KEEP` still decides whether a reading may *enter* the state; this is how much
of the state *leaves* it. Planted with no GPU and no footage, on his own -0.912
degrees, eight visits correlating / eight refused / eight correlating:

| | worst step, view px | steps over 10 px |
| --- | ---: | ---: |
| shipped gate | 46.69 | 3, after the arrival |
| filtered gate | 1.27 | 0 |

Delivered at the three banked downward views, `--bin band mode=trace`, 120
frames, applied step counts including every direction's first arrival:

| view | steps over 10 view px, main | with C1a | worst px, either |
| --- | ---: | ---: | ---: |
| down1 | 83 | 4 | 46.94 |
| down2 | 67 | 2 | 47.37 |
| down3 | 30 | 10 | 48.14 |

The 83 reproduces this memo's own 84. The steps that survive are arrivals, one
per direction, which C1a deliberately does not touch. **Excluding arrivals the
worst applied step at down3 goes UP, 27.75 to 42.02 view px**, and that is not
the gate: the band's own held reading steps 43.04 px there on every arm. C1a
fixes a gate artifact and a state jump is a different defect.

### 8.3 Increment 3: the C2 coverage gate answers yes

`--bin band mode=coverage`. Per direction, the visits the band accepted a
reading on, over 60 s of media time. His arc is azimuth 93 to 125 degrees,
12 cells here where `--bin refusals` counted 11 and read **1** of them.

| file | arc cells read | 10+ reads | 100+ reads | whole ring |
| --- | ---: | ---: | ---: | ---: |
| May-01, from his own 60 s | 10 of 12 | 10 | 8 | 91 of 128 |
| May-01, from the harvest's place 0 | 10 of 12 | 10 | 6 | 89 of 128 |
| Jul-25, bright undercast | 12 of 12 | 12 | 6 | 75 of 128 |
| Jul-14, the shimmer anchor | 12 of 12 | 12 | 12 | 128 of 128 |

**And it does not take minutes.** On May-01 eight of those cells are read
inside the first second and the next fifty seconds add little. Two cells, 92.8
and 95.6 degrees, are never read in any run of that file.

**The domain, because it decides what this licenses.** These are the band's own
`KEEP`. #171's accumulator stacked a far gate, a trimmed middle and a five-term
shape gate on top, so these are the most a live accumulator could ever see and
not what it would keep. A `no` would have killed C2; this `yes` licenses the
next measurement and nothing else.

### 8.4 Frame rate

`--bin playback`, 30 s, on the owner-box-class build. `main` and all three arms:
**898 redraws, 29.90 fps presented, 2 dropped, 0 starved**. No regression, and
the arms are not distinguishable on this instrument.

### 8.5 The null

Both toggles unset renders **byte-identically to `main`**: md5-equal 1024 px
frames at all three banked downward views through the unbent projection, and
md5-equal with the band LIVE over 40 frames. Each toggle moves the picture.

---

## 9. What shipped, and what it cost (2026-08-08)

Increments 1 and 2 plus the arrival staging are the app's **default
behaviour**. The three research environment variables are gone: not turned
off, deleted, along with the code they used to select. An A/B harness that
wants the old picture builds the old commit, which it can always do; a
configuration switch nothing reads is complexity with no reader.

### 9.1 The owner's verdict, which is why this shipped

`~/kjerag-ab/temporal-ab.sh`, three arms of one binary, six views, blind,
order balanced. **The bundle won or tied at all six.** He picked it at
`down1`, `down2`, `down3` and `shimmer`, said `good` was **same**, and at
`bad` picked `c3c1a` first and then added *"I like 3 on F too"*, which is the
bundle. On the round as a whole: *"it actually perceptually distorts a bit
less"*.

`down1` and `down3` **flipped** from round one, where `down1` read "same" to
him and `down3` read as baseline. The only thing that changed between the two
rounds is the arrival staging, which is the term the 2026-08-08 attribution
found and section 8 had not been written about yet.

### 9.2 The null is not "nothing moved". It is "what ships is what he saw"

This change moves the picture on purpose, so a byte-identity null against
`main` would be a failure rather than a pass. The null that means something is
against the **arm he answered on**: `--bin band mode=render count=40
size=1024`, the band live, md5 of the rendered frame, at all six A/B views,
under `seam=factory` and again under `seam=file`. **Identical at all twelve.**
Against the A/B's baseline arm - the one proven md5-equal to `main` - the same
twelve renders all differ, so the comparison is not reading one binary twice.

### 9.3 Delivered steadiness on the ship binary

`--bin band mode=snap count=300 size=1024 seam=file`, 21 probes, every step
the correction delivers on the across-seam axis. `main` is the same instrument
against the baseline arm, same box, same evening.

| view | 10+ px steps, main | ship | worst px, main | ship | 3+ px steps, main | ship |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| down1 | 73 | **1** | 56.4 | **11.5** | 247 | **32** |
| down3 | 115 | **2** | 56.4 | **12.5** | 418 | **85** |
| bad | 3 | **0** | 13.2 | **0** | 71 | **7** |

Every one of main's worst steps is an **arrival**: 15 of the 73 at `down1`,
19 of the 115 at `down3`, all at 56.4 view px, which is one direction's whole
correction switching on in one frame. The arrival class contributes no step
over 10 px anywhere on the ship binary.

**It is not zero, and section 8 should not be read as promising zero.** The
counts that survive are commits, one at `down1` at 11.5 px and two at `down3`
at 12.5, and the band's own held reading genuinely moves at those instants:
the cells' readings are bit-identical between the two arms (read rms 17.9,
p99 56.4 at `down1`), so what is left is a measurement changing and not a gate
artifact. A state jump is a different defect from a gate snap and this change
was never aimed at it.

### 9.4 What got worse: the comb

A dead neighbour cell zeroes a live cell's correction, so the corrected patch
has a hole in the middle of it rather than a lobe. Holding the gate up for
longer means more frames have one. Frames of 300 whose delivered field peaks
past 10 view px and comes back under 2 strictly inside its own reach:

| view | main | ship |
| --- | ---: | ---: |
| down1 | 24 | **40** |
| down3 | 45 | **100** |
| bad | 217 | **256** |

The `bad` figure reproduces the 257 the characterization measured with staging
on.

**And it is DEEPER as well as more frequent, which is a second change and not
a consequence of the first** (found in PR #172's review, 2026-08-08). The
attribution matters because it decides what the fix has to do. The gate on the
way out used to be `clamp(mix(confidence) / KEEP, 0, 1)` - the two cells'
confidences mixed and THEN clamped - and it is now each cell's own already
clamped `Cell.trust`, mixed. The clamp moved across the mix. So a live cell
beside a dead one used to be taxed by the pair's mixed confidence, which a
strong reading could carry over `KEEP` on its own, and is now taxed by the mean
of a 1 and a 0:

| the halfway ray, one cell at conf 0.95, its neighbour at 0.00 | main | ship |
| --- | ---: | ---: |
| tax applied | 0.731 | **0.500** |

which is 31.6 percent less correction in the notch. At the owner's `down1` pair
the notch measures 0.62 of the correction against 0.41, **34 percent deeper**.

Holding the gate up for longer is what makes more frames have a notch; moving
the clamp across the mix is what makes each notch deeper. **This is the next
build and it has to answer both.** A build that only shortens how long the gate
stays up would leave every remaining notch as deep as it is now. He was told
about it in the A/B briefing before he answered - *"If what you see is a stripe
across the corrected area rather than a jump in time, that is this"* - and
answered anyway; what he was not told is the depth, because it had not been
measured.

Not changed here, deliberately: the deeper notch is in the arm he picked blind,
so removing it would ship a picture he has not seen.

### 9.5 A correction to 8.1, found on the way in

Section 8.1's **3.46 degrees** is a prediction and not a measurement. It was
read by asking where a *linear* map would have to be for the curve to deliver
a tenth, which assumes the curve composes through the rest of the blend. It
does not: a lens's claim is its share times its own `landing.depth`, the two
lenses' depths are not equal, and the pair is renormalized after the curve.

Read directly off the weights the pass hands the fragment shader, on the
calibration fixture over 24 azimuths at the shipped 8 degree support
(`projection::tests::the_blend_curve_spends_less_of_the_handover_in_view`):

| crossfade | linear (`main`) | power 1.5 (ships) |
| --- | ---: | ---: |
| delivered 10-90, deg, mean | 4.85 | **3.85** |
| delivered 10-90, deg, per azimuth | 4.80 to 4.89 | 3.82 to 3.88 |
| share of the support | 0.61 | 0.482 |

So the delivered transition is **4.85 -> 3.85 degrees**, not 4.89 -> 3.46. It
is still inside the 3 to 4 degrees the memo asked for, and the support is
unmoved, which is what the exponent was chosen against. Nothing about the
owner's verdict changes: he judged the picture and the picture is the same one
this number describes.

**Re-measured 2026-08-08 in PR #172's review, and the first correction was
itself out by a spread.** This section shipped saying "4.88 to 4.92" and "3.88
to 3.92"; run over the 24 azimuths at 400, 4000 and 40000 grid steps the answer
converges to the table above and neither of those pairs is the mean or the
range. The linear column is the same test with the exponent at 1, where
`steepen` is the identity and the map is `main`'s.

### 9.6 The X2, which the A/B never exercised

All six A/B views are X4 Air. The blend curve has a consequence on the **ONE
X2** and it is a real one, and it is not the one this section shipped saying.

**What shipped, and why it was wrong** (PR #172's review, 2026-08-08). The fold
inequality is `BLEND_POWER * |disparity| <= FOLD * width`, and five things in
`band.rs` solve it for different unknowns: `carried`, `width`, `WIDEST_DEG`,
`reach` and `affordable`. The curve's gradient was divided into `carried`
alone. That made the two halves of stage 4 disagree - the width opened to
`|disparity| / FOLD` and the clamp only carried `FOLD * width / BLEND_POWER` -
so alignment the band had opened for was thrown away with nothing saying so: at
`KJERAG_HANDOVER_DEG=3` a 2.6 degree reading came out as 1.8, and at the X2's
own support the limit was 2.394 against a search that reads 2.6.

**What ships now: the division is in one derived constant, `SPEND = FOLD /
BLEND_POWER`, and all five read it.** `WIDEST_DEG` becomes `NEAR_DEG / SPEND` =
4.33 degrees, which is exactly the width at which the clamp equals the widest
reading the search can return, so **nothing the search can report is cut on any
camera at any handover width** and the clamp is back to being a guard on
arithmetic (`band::tests::the_band_carries_every_disparity_the_search_can_report`,
read at the shipped floor, the 2 degree fixture floor and the X2's own, with
`a_width_that_forgets_the_curve_throws_the_near_field_away` as its positive
control). The X4 Air family does not move at all: `affordable` answers
`2 * (half - NEAR_DEG)` in the roomy regime, which has no `SPEND` in it, so the
corpus's 9.24 to 9.82 are unchanged and so is every pixel of the six A/B views
(md5, 12 of 12).

**What the X2 pays instead, and it is a new disclosure.** `affordable` bounds
the FLOOR and `width` may open past it. Nothing could while `WIDEST_DEG` was
2.89, because the narrowest floor in the corpus was 3.99. At 4.33 a camera
overlapping by less than 9.53 degrees is under the line, and the X2 overlaps by
9.19 under its own fit: it affords 4.18, and a direction reading against the
near edge of the search opens its band to 4.33, which reaches 4.77 degrees off
the seam into 4.60 a side. The outer **0.17 degrees** of that corridor is
handed over by the coverage depth rather than by the crossover's ramp
(`band::affordable` says what that costs; it is not a fold and not a sample off
the end of the fisheye circle). It is live only where a direction reads inside
about 0.8 m.

So the X2's trade is: it now carries every reading whole where it used to give
up 0.206 degrees, and pays 0.17 degrees of corridor past its overlap at the
same instants. The alternative - keep clamping that camera, at 2.506 degrees
once `affordable` is consistent too - was available and was not taken, because
a clamp is silent and this is not. **Nobody has looked at an X2 under either.**
It is flagged for the owner rather than buried here.

The fold itself is still tested from both sides
(`projection::tests::the_blend_curve_cannot_fold_the_narrowest_camera` measures
the curve's peak gradient and shows the undivided limit folding that camera).
