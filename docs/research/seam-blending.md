# The seam's photometric handover, and why one gain cannot make it look right

**Status:** measured verdict, no fix proposed as built. **Date:** 2026-08-01.
**Scope:** the *perceptual* handover at the seam - colour, brightness, any and
all. Geometry is out of scope by the owner's ruling: *"seam offset / rotation /
translation / warping etc out of scope."*

Everything below is measured. The footage is the owner's own flights; no
filenames, serials or camera keys appear here, and the captures the tables were
read off stay in gitignored `scratch/`.

---

## TL;DR

At the owner's own wide reference view the seam shows a **6.5 code step on 17
to 24 code content - 28 to 38 percent** - and it is **brightness, not colour**:
the three channels step within one code of each other. It is locked to the seam
at three instants nine seconds apart and at two yaws, and its width is the
crossover's.

Issue #103 stage 7 (PR #138) moves it by about 0.7 codes of 6.5. **Not because
it is tuned wrong, but because three properties of the design put the artifact
outside what it can reach**, and each of the three is measured below:

1. the correction is **multiplicative** and the difference is **additive**;
2. the estimator weights **brightness squared**, so the directions where the
   artifact is visible carry about one percent of the weight;
3. the loss is **in codes**, and the eye reads **ratios**.

None of the three is a bug. Each was chosen on a measurement, and each
measurement was made on content where the artifact does not show.

---

## 1. What the eye sees, measured

The owner's wide reference is a sunset over ploughed soil at a field of view of
114 degrees. Profile of the drawn picture against angle from the seam plane,
restricted to a window on the soil so that strips at one distance hold one kind
of content, mean codes of 255:

| degrees from the seam | R | G | B |
| ---: | ---: | ---: | ---: |
| -8 | 17.3 | 12.4 | 15.5 |
| -4 | 17.4 | 12.4 | 15.8 |
| -1 | 17.5 | 12.4 | 16.1 |
| **0** | 18.0 | 12.7 | 16.5 |
| **+1** | 20.8 | 15.1 | 19.3 |
| **+2** | 24.0 | 18.0 | 23.0 |
| +4 | 24.1 | 18.4 | 22.7 |
| +8 | 24.8 | 19.4 | 23.4 |

Flat, then a rise inside the crossover, then flat. The step is **+5.9 / +5.0 /
+6.3 codes**, and the spread between the channels - the hue step, which is what
stage 7 corrects - is **under one code**.

**It is the handover and not the scene**, and that is the control rather than an
assertion. The same window, at three instants over nine seconds of film and at
a yaw 28 degrees away so the seam crosses a different part of the field:

| run | R | G | B |
| --- | ---: | ---: | ---: |
| the reference instant | +5.88 | +5.04 | +6.30 |
| 3.2 s later | +6.07 | +5.04 | +6.00 |
| 9.2 s later | +4.85 | +4.18 | +5.42 |
| the reference instant, yaw -100 | +6.33 | +5.36 | +6.82 |

A scene edge moves with the film and turns with the view. This does neither, it
sits at zero degrees every time, and its width is the width the pass mixes the
lenses over, which was two degrees when this was measured and is eight since
2026-08-05.

In the picture, stretched sixteen times about the soil's own level with the
seam plane and the crossover drawn on: `scratch/stage7/evidence-may-stretched.png`.

## 2. Why a gain cannot reach it

A step of 6.5 codes on content at 21 codes needs a gain of **1.35**. Three
things follow immediately and each is fatal on its own.

- `band::LIMIT_LN` is 0.25, which is a gain of 1.28. The guard refuses it, and
  the guard is right: it is four times the widest gain ever measured on any
  capture.
- The same 1.35 applied to the sky in the same frame, at 190 codes, moves it by
  **66 codes**. There is no single multiplier that fixes the soil and leaves the
  sky.
- The pooled gain the pass actually drew this view with is 0.4 to 1.4 percent,
  which at 21 codes is **0.3 codes** against a 6.5 code step.

Fitted over the whole ring at that instant, on the same readings, what each
model leaves:

| channel | nothing | a gain | an offset | both |
| --- | ---: | ---: | ---: | ---: |
| R | 6.79 | 6.79 | **6.47** | **6.08** |
| G | 5.18 | 5.17 | **5.00** | **4.39** |
| B | 6.13 | 5.76 | **4.70** | **4.41** |

Codes. **An offset beats a gain in every channel, and the pair beats both.** The
offsets are -2.1 / -1.4 / -3.9 alone and -4.8 / -4.3 / -5.9 beside a gain.

That is the additive term stage 3 measured, attributed to near-field alignment
and declined (6.10), and that stage 7 re-measured on flat bright content and
found indistinguishable from a gain (6.11). Both of those readings stand. **What
neither of them measured is dark content**, which is where an additive term is
the whole of the difference and a multiplicative one is nothing.

## 3. Why the estimator cannot see it

The pooling weights every direction by its own brightness **squared**
(`pooled_gain`, and the same weight inside the ring fit). That is not an
oversight: it was measured in across nine captures, and an equal-weight average
of log ratios is worse than doing nothing on four of them.

But brightness squared means a direction on 20-code soil carries
`(20/190)^2 = 1.1 percent` of the weight of one on 190-code sky. **The pass is
fitted almost entirely on the content where the artifact is invisible, and
almost not at all on the content where it is 38 percent.**

## 4. Why the loss is the wrong one

Stage 3 chose least squares **in codes** because it left the smallest step *in
codes* on all nine captures. On this view that choice reads:

| | soil, 21 codes | sky, 190 codes |
| --- | ---: | ---: |
| a 6.5 code error is | **31 percent** | 3.4 percent |

An eye judges a step against what it is a step of. A loss in codes is a loss
that treats those two as the same size and therefore spends its whole budget on
the second one. **The metric stage 3 was scored on and the metric the owner is
looking at are not the same metric**, and every choice downstream of it -
including stage 7's - inherits that.

## 5. What sharpens it: the handover is fixed in angle, not in pixels

The crossover is a fixed angle whatever the view: two degrees when this was
written and eight since 2026-08-05, when the owner chose the wider handover
label-blind (docs/ROADMAP.md). On a 1024 wide render, both:

| field of view | 2 deg was | 6.5 codes across it was | 8 deg is | 6.5 codes across it is |
| ---: | ---: | ---: | ---: | ---: |
| 20 | 102 px | 0.06 codes/px | 410 px | 0.016 codes/px |
| 60 | 34 px | 0.19 codes/px | 137 px | 0.047 codes/px |
| **114** | **18 px** | **0.36 codes/px** | **72 px** | **0.090 codes/px** |

The owner's complaint arrived at 114 degrees. The same residual difference is
five times sharper there than at the view stage 5 was judged on, because the
handover's width in pixels shrinks as the view widens. Nothing about the
correction changed; the regime did. Widening the crossover to 8 divides every
codes-per-pixel entry by four without changing that shape, so section 9's move
4 below is chosen against the left-hand pair and priced against the right.

## 6. What is not the problem here

- **Not colour.** The hue step is under one code at this view, on content where
  the brightness step is 6.5. Stage 7's per-channel work is real - it takes the
  hue step from 3.4-5.6 codes to under a code on his captures - and it is not
  what this view is about.
- **Not geometry.** Out of scope by ruling, and separately: the two April
  reference views' skies show no photometric seam at six times contrast
  (`scratch/stage7/sky-apr1-warm.png`, `sky-apr2-warm.png`), so whatever is
  wrong there is on the axis the owner excluded.
- **Not the band's alignment.** The step is flat either side and rises only
  inside the crossover; a misregistration on flat soil is worth 0.33 to 0.76
  codes at the residual the pass leaves (6.11).

## 7. What the design could reach, and at what price

None of these is built and none is recommended here; the owner decides. Each is
priced against the machinery that already exists.

| option | what it changes | cost | risk |
| --- | --- | --- | --- |
| **A. Weight the estimator by what the eye reads** rather than by brightness squared - relative error, or a perceptual space | one line in `pooled_gain` and the ring fit; no new state, no new constant | free | dark patches are noisier, so the gain gets noisier; needs the flicker column re-run |
| **B. An additive term beside the gain**, per channel, fitted where it shows | three floats of state, one guard, one owner decision about black level | ~0.05 ms | it moves a hemisphere's black level, which the owner reserved on; measured here it is what the artifact IS |
| **C. Spread the residual wider** than the crossover, using the fade `Tint` already has | no new machinery, one width | free | a wide low-frequency correction is a halo if it is wrong |
| **D. Per-direction photometry** instead of one number for the ring | a per-cell correction and its own smoothing | ~0.1 ms | the artifact stage 5 measured as scalloping is the same class of risk on this axis |

**The honest summary of the four:** A and C are free and inside the current
architecture. B is what the measurement above actually points at and is the one
that needs an owner ruling. D is the largest change and the least indicated by
anything measured so far.

## 8. The process finding

Stage 3 measured the exposure step on the seam ring and chose its estimator on
nine captures. Stage 7 measured the colour step on the seam ring and chose its
correction on nine captures. **Both were scored on the ring's own statistic, and
the ring is dominated by bright content because that is what the weighting
says.** The owner has now twice reported an artifact that is not in the ring
statistic: first a hue the ring statistic could not represent, and now a
brightness step on the darkest content the ring carries.

The rule that follows is stage 6's own, on the other axis: **acceptance in
picture space, at the view the complaint was made at, on the content the
complaint is about.** A photometric acceptance number taken over a whole ring is
an average over content, and an average over content is exactly what hides a
defect that lives in one part of the range.

---

> **STATUS, 2026-08-01: the APPLICATION described below was built, rejected by
> the owner, and REMOVED. What shipped from it is nothing; what survives is the
> measurement layer and the findings.** He tested it twice: *"I dont think its
> aggressive enough with blending"*, and then, after the wide form, *"Honestly
> the 7+8 seam looks worse than before... weird artifacts extending down and up.
> I don't think this approach is valid."* Sections 9 to 13 describe machinery
> that no longer exists in the tree; they are kept because the measurements in
> them are true and were expensive, and because section 15 only means anything
> against them. Section 16 is what the whole thing is worth.

# Stage 8: what was built, and what it is worth

**Status:** built and measured. **Date:** 2026-08-01. Everything above is the
verdict this stage was written against and it is left standing; this half is
what answering it cost and what answering it bought.

## 9. The five moves, and which measurement each one comes from

| # | what | the measurement it answers |
| --- | --- | --- |
| 1 | **ratio space**: the estimator's loss is the residual divided by the level it sits on, per channel | section 4 - the same 6.5 codes is 31 percent of soil and 3.4 percent of sky, and a loss in codes spends its whole budget on the second |
| 2 | **a gain AND an offset**, fitted jointly | section 2 - an offset beats a gain in every channel and the pair beats both; a gain that could reach the soil would move the sky by 66 codes |
| 3 | **the offset is per direction**, not one number and not a five-term shape | section 12 below - the ring's residual after a constant, one cycle and two is 4.2 to 5.5 codes rms against a frame noise of 0.8 to 1.0 |
| 4 | **one width**, in pixels of the delivered view, gated by what a wider handover would cost | section 5 - the crossover was two degrees when this was chosen, 102 pixels at fov 20 and 18 at fov 114; at the 8 it is now, 410 and 72 |
| 5 | **a profile with no corner**, and dither inside it | the residual physics: a corner in a gradient is a Mach band and an 8-bit ramp of a fraction of a code per pixel is a staircase |

Sequential fitting was tried and refused on arithmetic before it was built. A
gain fitted alone in ratio space on this data comes out at **1.15**, because
equal weight in logs is pulled by the dark end; it then leaves the sky 26 codes
wrong and the offset step cannot recover it. Fitted jointly on the same two
points the answer is a gain of **0.973 and a lift of 7.1 codes**, which
reproduces both ends exactly. Glare is a gain slightly under one plus a lift,
and only a two-parameter fit can see that.

## 10. The instrument: what a seam is worth to an eye

`kjerag-spike --bin colour mode=profile`, the picture-space half, now reports
**the steepest local Weber contrast across the seam**, at lags of 1, 2, 4, 8, 16
and 32 pixels **of the delivered view**. Two properties are deliberate:

- **every pair it maximizes over straddles the seam**, so the statistic is
  about a handover and not about a scene; the decoy great circle says what the
  scene contributes;
- **the bins are one pixel of the view being judged**, not one degree, because
  the same residual is five times sharper at fov 114 than at fov 20 and that is
  the whole of section 5.

**Controls, and they are the same code path over a different picture:**

| the control | what it has to read | what it reads |
| --- | --- | --- |
| a flat field, ratio 1.00 | 0 at every lag | 0.000 percent at every lag |
| a flat field, ratio 1.02 over one pixel | 1.980 percent at every lag | 1.980 percent at lags 2 and up |
| a flat field, ratio 1.05 over one pixel | 4.878 percent | 4.878 percent at lags 2 and up |
| the same 1.05 spread over 64 pixels | the same whole step, `lag / 64` of it locally | step 4.60, lag 1 **0.077**, lag 32 **2.465** |

The last row is the whole claim of stage 8 in one line: a step and a ramp of the
same size are different artifacts, and this instrument can tell them apart to
three decimal places.

## 11. What it reads at the owner's own views

Weber contrast, worst channel, at the steepest lag; **before** is the same
branch with the photometry held off, so the two differ by this stage and by
nothing else.

| view | before | after |
| --- | ---: | ---: |
| the May wide view, on the soil he complained about | **42.3 percent** | see the acceptance table in the PR |
| the same view, on the sky at the seam | 6.7 | " |
| a second May view | 8.3 | " |
| his own sun-in-one-lens reference | 8.0 | " |
| a corpus X4 with the sun in one lens, another shooter | 11.7 | " |
| an April geometry view, out of scope by ruling | 4.5 | " |

## 12. Why the offset is per direction

Measured on the owner's own reference instant, over 72 azimuths and four
frames, what a correction of each shape LEAVES round the ring, in codes rms:

| channel | nothing | a constant | one cycle | two cycles | frame noise |
| --- | ---: | ---: | ---: | ---: | ---: |
| R | 6.73 | 6.41 | 5.60 | **5.51** | 1.00 |
| G | 5.13 | 4.95 | 4.23 | **4.20** | 0.82 |
| B | 6.08 | 4.63 | 4.40 | **4.16** | 0.92 |

The five-term basis stage 7 fitted through takes 18 percent off R and leaves
**five and a half times the noise floor**. What varies round a seam is not a
low-order shape, so stage 7's field is deleted rather than extended and the
correction is the reading at the direction it was read at.

**A hole the size of the complaint.** Stage 7 read a photometry only where the
correlation had established what content it was looking at, and on this footage
that left **50 of 128 directions with no colour at all, in a continuous arc**.
A refused correlation now reads at the calibration's own shift and is believed
at the price of being wrong there, which is the same number the width is gated
by: the content's own gradient times the angle the pass cannot correct. On the
owner's wide view that one change is most of the improvement.

## 13. What is left, and it is not a step any more

The correction is carried across the handover and eased to nothing by the angle
the two lenses stop sharing a picture at, because a player may not move a
hemisphere's black level. What that shape leaves is not an edge but a **ramp**:
the two hemispheres still differ by what they differ by, and the picture walks
between them over several degrees instead of stepping between them over one
pixel. On the owner's wide view the drawn profile goes from a hard 7-code step
inside 2 degrees to a smooth walk with a flat plateau across the seam.

The remaining reading is that ramp, and it is what every lag over 8 pixels is
now measuring. Widening the taper past the overlap would halve it again and is
**not** built: it is the halo risk of option C above, and it would put a
low-frequency correction on a picture where nothing can check it.

---

# Stage 8, second form: symmetric wide matching, and who draws the line

**Date:** 2026-08-01, after the owner viewed the first form twice.

## 14. The ruling, and why the shape it replaced was wrong

His first verdict: *"I dont think its aggressive enough with blending. For
example smoke3-2-drawn."* The first form carried the correction across the
handover and eased it to nothing **by the overlap**, seven degrees off the seam,
on the argument that past there "how these two lenses differ here" is not a
statement anything can check. What that leaves is the whole correction ramped
over four degrees, and a ramp of the whole correction is a patch difference -
which is what he was looking at.

**The symmetric split dissolves the objection the shape was protecting
against.** The reservation was that a player may not move a hemisphere's black
level. It does not: each hemisphere moves **half** the mismatch towards the
other, which is precisely the argument stage 3 used to split a gain between two
hemispheres, applied to an offset. So the correction is carried **to the pole**,
which is the only end that is not a taste - an azimuth is what the field is read
at, and a pole has none, so a field carried to one has to arrive single-valued.

Measured at his own wide view, the drawn profile from 8 degrees one side to 8
degrees the other:

| | at -8 deg | at +8 deg | apart |
| --- | ---: | ---: | ---: |
| the photometry held off | 17.9 | 25.5 | **7.55 codes** |
| the first form, eased out by the overlap | 17.9 | 25.5 | 7.55 |
| **carried to the pole** | 19.7 | 22.6 | **2.93 codes** |

**The halo did not appear.** It was the priced risk of option C and the reason
the first form stopped at the overlap. Over the same window the long-lag Weber
contrast goes **44.7 percent to 24.4 at 64 pixels and 49.2 to 26.8 at 128** -
the wide matching does not add a low-frequency artifact, it removes one, and it
removes it by 45 percent. A symmetric half-correction has half the excursion per
hemisphere by construction, and it is spread over eighty degrees instead of
four.

**A count of pixels came out with it.** The first form asked the handover for
`SPREAD_PX` pixels of the delivered view. It decided nothing at any field of
view the player offers - the optics' ceiling or the content's own price is
always reached first - and the one place it bit, it made the handover narrower
than the content would have borne. Deleted, with its constant and with the
screen-space derivatives that fed it.

## 15. Who draws the line that is left

His second verdict: *"To the eye, it still effectively looks like a line."*

A line at one pixel has two possible authors and they are separable. A
**photometric** step is a difference in LEVEL: it shows on content with no
gradient in it at all, it is at the seam and nowhere else, and a photometry
moves it. A **misregistration** is a difference in POSITION: it shows only where
there is content to draw twice, at the lag its own size in pixels puts it at,
and no photometry can touch it.

The instrument runs the same statistic straddling a line a few degrees off the
seam, in the same window and the same content. Weber contrast, worst channel,
**excess over what that content reads anywhere**:

| view | 1 px | 2 px | 8 px | 32 px | verdict |
| --- | ---: | ---: | ---: | ---: | --- |
| the owner's wide view, soil | **+0.87%** | +0.61% | -3.33% | +7.90% | under the JND at an edge's own scale |
| the smoke view he named, whole frame | **-0.73%** | +0.94% | +0.95% | +18.17% | no line the content does not read everywhere |
| the fov 30.6 view he sent next | **-0.82%** | -0.69% | -0.88% | +0.23% | no line at any scale |
| the sky at the seam | -0.07% | +0.23% | +0.37% | +2.75% | no line |
| **his own gear at the seam** | **+5.87%** | +5.73% | +6.22% | +7.91% | **a line, and the photometry moves it by 0.00** |

The last row is the positive control this decomposition needs, and it is the
answer. At that azimuth the seam reads six percent above the same content
elsewhere at every lag, and turning the whole photometric stage on moves it from
**5.94 to 5.94 percent**. A photometric correction cannot reach it because it is
not photometric.

**The verdict: after stage 8 the photometric author is at or under the
just-noticeable difference at the one and two pixel lags on every reference view
the owner has given, and what still reads as a line is GEOMETRIC.** It lives
only where there is content to misregister, it is unmoved by any photometry, and
the fov 30.6 view sharpens it: at 0.0064 degrees per pixel a photometric step
would still be a step, and there is none, while a fifth of a degree of
misregistration is thirty pixels there and shows at the lags a thirty-pixel
feature shows at.

That makes the local-warp-versus-pose decision the true blocker of "no line",
and it is not this stage's to make. **No local warp is built here.**


---

## 16. What this cost, and the one rule that comes out of it

**The application is gone. The instruments stay.** PR #138 ends as a
measurement-only change: the shipped crates are main's, byte for byte, and the
whole branch is one instrument file and this record.

**The process finding, which is the expensive part and the durable one.** Every
acceptance statistic this campaign has ever used STRADDLES THE SEAM. Stage 8
noticed that the statistic was in the wrong units and fixed that, and the
replacement straddled the seam too. So nothing ever measured what an applied
correction does to the picture it is painted OVER, and the owner rejected two
builds on an artifact class the entire acceptance layer was structurally unable
to see. A per-direction field applied over wide spatial support paints each
direction's own noise along that direction's whole sweep: it is stage 5's
scalloping on the photometric axis, and stage 5's own lesson did not transfer
because nobody had written it down as a rule about FIELDS rather than about
geometry.

**The rule: a field that is applied over an area is accepted on the area, not
on the boundary.** Any correction with spatial support owes two numbers - what
it does at the seam, and how smooth it is everywhere else - and the second one
needs its own instrument with its own plants. That instrument now exists
(`kjerag-spike --bin colour`, the interior block), it is registered as the
anti-acceptance for photometric work in docs/research/reference-views.md, and
it separates the three builds cleanly:

| build | interior roughness | worst neighbour step |
| --- | ---: | ---: |
| main, as shipped: one gain over the whole ring | **0.03%** | 0.18% |
| the salvage: a five-term shape that cannot stripe | 0.53% | 2.26% |
| **the rejected build: per direction, wide support** | **1.01%** | 1.99% |
| its own null: nothing applied at all | 0.000% | 0.000% |
| a planted 0.5-code ripple, eight cycles | 2.07% | 0.69% |
| a planted 2.0-code ripple | 8.27% | 2.76% |

**And one finding that is not affected by any of it**, because it is about the
pictures and not about the correction: at every reference view the owner has
given, the residual line's excess over what the same content reads a few
degrees away is at or under the just-noticeable difference at the one and two
pixel lags, while at the azimuth his own gear crosses the seam it is +5.87
percent and turning the entire photometric stage on moved it from 5.94 to 5.94.
**What still reads as a line at the seam is geometric.** That is the foundation
of the local-warp-versus-pose decision, and it is the campaign's next question.

---

# Stage 10, step P.1: the deterministic normalization, measured and refused

**Status:** measured verdict, application built, switched OFF by default, not
recommended. **Date:** 2026-08-09. **Instrument:**
`kjerag-spike --bin expose mode=meta`, which is new and is what this section is.

The plan of record for stage 10's first step was a **deterministic per-lens
exposure normalization from file metadata**: read the trailer's own per-frame
per-lens shutter, ratio-normalize the two lenses before fusion, and be
structurally unable to repeat stage 7/8's noise-painting because nothing is
estimated from pixels. The attraction is real and the reasoning is sound as far
as it goes. It is also, measured, **the largest single regression this campaign
has produced**, and it is refused here on nine of the owner's own reference
views.

## 17. What the file actually carries

Verified on the owner's own captures rather than quoted, because the "check
whether an ISO is in there" was half the task.

| record | what it is | per lens? |
| --- | --- | --- |
| **4 and 12** | `{u64 ts, f64 shutter_s}`, one sample per frame | **yes** - 4 is lens 0, 12 is lens 1 |
| **9** (`AAAData`) | 48 bytes a sample, `ts_ms, 0, 0, 0, 0x02000000, a, b, 0, 0, 0, 0, 0` | **no** - one track for the file |
| **1** (protobuf) | 55 keys, calibration and clocks | no exposure key at all |
| 11, 22, 27, 28, 29 | undecoded | none has a second copy for a second lens |

On the May file record 9 is 2077440 bytes, which at 48 is 43280 samples for a
capture whose two shutter records hold 43959 each: one per frame, written once,
`a` in 2040 to 2161 and `b` in 6343 to 6349. **There is no second `AAAData` for
the second lens**, so whatever `a` and `b` are, they cannot complete a per-lens
sum. That reproduces the 2026-07-31 reading (insv-format.md 2) on a different
capture, three firmware generations along.

**So the answer to "is there a per-lens ISO or gain" is no, and it is no by
absence and not by ambiguity.** Shutter is the only per-lens exposure quantity
the format carries.

**And on the ONE X2 there is not even that.** `VID_20251018_191318_00_002.insv`
writes record 4 (131696 bytes, 8231 samples) and **no record 12 at all**. One
lens's shutter and nothing to divide it by: on that camera the step is not
merely uninformative, it is unavailable, and `--bin expose mode=meta` refuses
the file in those words.

## 18. Does the shutter ratio predict the artifact? No, and it is not close

`--bin expose mode=meta` asks the one question the whole design rests on. Per
frame it reads `g`, the trailer's own `shutter1 / shutter0` at that frame's own
camera instant, and the delivered ratio, the two lenses' mean luma over the
overlap annulus - the same world directions in both, so a brightness and not a
content difference, which is the 6.3 estimator kept deliberately so the two
readings are comparable. A correction multiplies the lenses by `sqrt(g)` and
`1/sqrt(g)`, so what it LEAVES is `ln(delivered) - ln(g)`, and that is the last
column.

Nine reference views, five X4 Air captures, 40 to 60 consecutive frames each:

| view | `g` says | the lenses actually differ by | correlation | what the correction leaves |
| --- | ---: | ---: | ---: | ---: |
| the dirt reference, 488.855 | -34.9 to -32.9% | -2.4 to -1.1% | +0.51 | **+2038%** |
| the May wide view, 630.763 | -37.4% | -3.6 to -3.2% | 0.00 | +1163% |
| the fov 30.6 view, 669.369 | -36.7 to -32.8% | -4.3 to -1.5% | +0.19 | +1283% |
| the green cast, 594.027 | +48.5 to +52.0% | +2.8 to +3.7% | **-0.34** | +1070% |
| the green cast, 602.368 | +46.8 to +57.0% | +1.5 to +4.0% | **-0.67** | +1227% |
| GOOD and BAD, 50.117 | +50.0 to +52.4% | +0.07 to +0.78% | **-0.74** | +7689% |
| down1, 65.666 | +42.2% | -0.26 to +0.26% | 0.00 | +22686% |
| the shimmer view, 36.303 | +39.1 to +47.1% | +0.07 to +4.3% | +0.34 | +1375% |
| hard mode, 31.064 | +38.6 to +45.5% | -0.65 to +4.1% | +0.18 | +1780% |
| the X2, 77.978 | *no record 12* | | | *refused* |

Three readings and each is fatal on its own.

1. **The size is wrong by a factor of twenty.** The metadata says the two lenses
   are 33 to 57 percent apart. Their pictures of the same directions are 0.07 to
   4.3 percent apart. A correction has to be the size of the artifact to be a
   correction of it.
2. **The sign is not even reliable.** The correlation over nine views runs -0.74
   to +0.51 and averages about -0.05. **Three of the nine are strongly
   negative**, which is metadata pointing the wrong way, and the two most
   negative are the owner's own green-cast pair and the GOOD/BAD instant.
3. **It never once helps.** The last column is never below 100 percent on any
   view. The best case makes the artifact **eleven times** bigger and the worst
   **two hundred and twenty-eight times**.

**Why, and it is not a tuning problem.** The two lenses run independent
auto-exposure loops that trade shutter against sensor gain to reach the same
picture brightness. `g` therefore measures how differently the two hemispheres
are **lit** - on a paraglider, sun against ground, and genuinely a factor of 1.8
- and not how differently they came **out**, which is a percent or three because
the camera has already corrected for the first thing. **Normalizing by shutter
does not remove an error; it removes the camera's own correction and puts the
raw lighting difference back into the picture.** Completing the sum would need
the matching per-lens gain, and section 17 is the search for it.

This is the 2026-07-31 finding (insv-format.md 6.3, ROADMAP the same day) with
the same verdict and a wider corpus: it was 4 to 20 times worse on two captures
then, and it is 11 to 228 times worse on nine views of five captures now, at the
owner's own reference aims, on the flat-seam architecture he approved.

## 19. What it does to the picture, at the view the complaint was made at

The dirt reference, `time=488.855 yaw=-5.17 pitch=2.56 fov=218.99 lock=1`, eight
frames, 1024 px. `off` is `main`'s picture, proven byte-identical (section 21).

| | the seam's step, `--bin expose mode=render` | the decoy circle |
| --- | ---: | ---: |
| off | **-2.143 codes** | -2.164 |
| on | **-15.261 codes** | -1.993 |

**2.14 to 15.26 codes, seven times worse, and the decoy does not move**, so the
thirteen codes are the correction's and not the scene's. For the ladder this
line carries: stage 3's far-field gain took the same step **2.265 to 1.424
codes**, and this step takes it to 15.26. What P.1 delivers on top of the pooled
gain is **minus six code-lengths of it**.

The eye's own metric, `--bin colour mode=profile`, same view, worst channel:

| | step in codes | Weber, worst lag | 1 px excess | 2 px excess |
| --- | ---: | ---: | ---: | ---: |
| off | 4.359 | 25.67% (128 px) | +0.61% | -0.03% |
| on | **17.109** | **48.58%** (128 px) | +0.97% | +0.76% |

The standing bar is the one and two pixel excess at or under a one percent
just-noticeable difference. Both arms are still under it, because **the
correction is a step in LEVEL laid across an eight degree handover and an eight
degree handover at fov 219 is 72 pixels wide**: what it makes worse is every lag
from 8 pixels out, which is the ramp, and it makes the worst of them roughly
twice as bad. It is not a defect the one-pixel bar can see, which is precisely
the process finding of section 16 read from the other side.

## 20. The interior, and what the instrument can and cannot say about it

The binding area-acceptance rule (section 16): a field applied over an area is
accepted on the area. `--bin colour`'s interior block, 7 to 60 degrees off the
seam, same view:

| build | interior roughness | worst neighbour step |
| --- | ---: | ---: |
| off | 0.07% | 0.69% |
| **on** | **0.05%** | **0.29%** |
| the rejected stage 8 build, for scale | 1.01% | 1.99% |
| a planted 0.5 code ripple | 1.37 to 1.42% | 1.16 to 2.14% |

**It passes, and the honest reason is that it cannot fail.** A metadata gain is
one scalar for the whole frame, with no azimuthal structure of any kind, and the
statistic is the residual after a five-term harmonic - which absorbs a constant
exactly, at the zeroth term. **State the caveat with the number**: that
instrument compares the band held against the band drawn, and both of its arms
carry this term, so the term cancels inside it and the 0.05 above is a reading
about the pooled gain and not about P.1. What says P.1 cannot stripe is the
structure of P.1 and not this measurement.

This is the one gate the step passes, and it passes it for a reason that makes
the pass worth nothing: the failure mode it guards against is not the failure
mode this step has.

## 21. The pooled gain does not absorb it, and cannot

The stage-10 plan expected the shipped stage-3 gain to have absorbed the static
part of the exposure difference already, and asked for a re-fit so the two would
not double-correct. **Measured, there is nothing to re-fit, and the reason is a
finding of its own.**

The shipped gain reads **`+0.00287` ln, evidence 0.032, on both arms, to five
decimals**. It does not move when the normalization is switched on, and it will
never move, because the band's `measure` and `pool` compute entry points sample
the **decoded planes**, which is upstream of the fragment shader where both
`tone_split` and the new `exposure_split` are applied. The pooled gain therefore
cannot see the metadata term at all.

So the two do not double-correct. They **multiply blindly**: the pass applies
0.287 percent of measured correction and then 33 percent of unmeasured one on
top of it, and nothing in the loop ever learns that the picture it produced is
now a third of a stop out. A correction the measurement layer is structurally
blind to is exactly the class of thing this campaign has now been burned by
three times, and it is worth writing down as a property of the architecture
rather than of this step: **anything applied in the fragment shader is invisible
to the band, because the band measures the source and not the picture.**

## 22. The verdict, and what ships

**The step is refused. The application is built and switched off; the
instrument, the trailer verification and this record are what the branch is
for.** That is PR #138's ending and it is deliberate: the numbers are the
deliverable, and a number is worth more when the arm that produced it can be
re-run.

`KJERAG_EXPOSURE_NORM=on` draws it. Off is the default, off is what an empty
string reads as, and off is `main` byte for byte at four registry views
(`down1` 7d2200ea, `down3` a19a9b80, `bad` f27874ed, `shimmer` 54fc67b7,
`--bin null`); on moves `down1` to 7d2ef9bc, which is the positive control that
the mechanism is live.

**What would change the verdict, stated so nobody has to re-derive it.** A
per-lens **gain** - ISO, analogue gain, digital gain, anything that closes
`exposure = shutter * gain` for each lens separately. It is not in records 4, 9,
12 or 1, it is not in the undecoded records in any per-lens shape, and it is not
in the Osmo 360's telemetry either (section 23). Until it exists in a file,
deterministic exposure normalization is not an under-tuned idea, it is an
under-determined one.

## 23. The Osmo 360 `.OSV`: per frame, yes; per lens, no

The stage-10 plan asked the same question of DJI. **An `.OSV` carries per-frame
exposure metadata, in more detail than an `.insv` does, and none of it is per
lens.**

Sample 0 of each `djmd` track is the calibration message (`crates/meta/src/osmo.rs`
on `feat/osmo-osv`); every sample after it is a per-frame block, one per video
frame, and inside it:

| field | what it is | how that was established |
| --- | --- | --- |
| `3.2.3.1` | **ISO**, `f32` | tops out at exactly **800.0** on the four captures whose own filenames say "iso max 800" and at exactly **1600.0** on the one that says "iso maxed 1600" |
| `3.2.4.1` | **shutter**, a `(numerator, denominator)` rational | the denominator bottoms out at exactly **100** on both captures whose filenames say "shutter max 1/100", and nowhere else |
| `3.2.6.1` | white balance in kelvin | 4743 to 6443 across the corpus |
| `3.2.16.1` | a temperature in degrees, 41 to 45 | unidentified |

The identification is the corpus's own named caps and not a guess, which is why
it is worth having: the owner shot those files with the caps written into their
names.

**But both `djmd` tracks carry the same block.** Over 584 to 899 consecutive
frames of each of seven captures, the exposure product `ISO * shutter` between
the two tracks reads a ratio of **0.99966 to 1.00067 mean, 0.00044 to 0.00288
standard deviation, with 82 to 98 percent of frames identical to within a
thousandth**. On the two Dlog-M captures they never differ at all.

Where they do differ it is a **write-time race on one shared value and not two
metering loops**, and the evidence is that the differences appear only while the
value is moving: 56 of 58, 17 of 17 and 142 of 187 of them sit on a frame where
the track's own value is changing, and the sign is one-way (one track lower on
54 of 58 and on 140 of 187). One capture, `CAM_20250715191201_0003_D.OSV`, is
the exception - 48 of its 95 differences are on steady frames and reach 3
percent - and it is **not** explained here; it is the smallest and oldest file
in the corpus and it is flagged rather than fitted.

**So the answer for `.OSV` is the answer for `.insv` with one fewer step: there
is no per-lens exposure to ratio, so there is nothing deterministic to
normalize.** No implementation is proposed, and none would have been possible on
this branch in any case: `main` does not play `.OSV` at all - it is refused by
name (`crates/meta/src/format.rs`, "That is a DJI video. Kjerag plays Insta360
.insv only.") - and the reader that would change that is the unmerged
`feat/osmo-osv`.
