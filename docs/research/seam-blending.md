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
