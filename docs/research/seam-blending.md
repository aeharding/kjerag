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
sits at zero degrees every time, and its width is the two degrees the pass mixes
the lenses over.

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

The crossover is two degrees whatever the view. On a 1024 wide render:

| field of view | two degrees is | 6.5 codes across it is |
| ---: | ---: | ---: |
| 20 | 102 px | 0.06 codes/px |
| 60 | 34 px | 0.19 codes/px |
| **114** | **18 px** | **0.36 codes/px** |

The owner's complaint arrived at 114 degrees. The same residual difference is
five times sharper there than at the view stage 5 was judged on, because the
handover's width in pixels shrinks as the view widens. Nothing about the
correction changed; the regime did.

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
| 4 | **one width**, in pixels of the delivered view, gated by what a wider handover would cost | section 5 - two degrees is 102 pixels at fov 20 and 18 at fov 114 |
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
