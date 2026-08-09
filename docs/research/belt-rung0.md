# Rung 0 of the belt: the clock, taken before the build

**Status:** a design-time measurement and a recommendation. Nothing is built,
nothing is switched on, and no product code is touched. **Date:** 2026-08-09.
**Audience:** the owner and whoever writes the belt, as the gate that has to be
passed before a line of it exists.
**Instrument:** `crates/spike/src/bin/belt.rs`, on `research/belt-rung0`.

**Why this exists at all.** Issue #171 proved a mechanism and was refused on a
clock nobody had measured first: *"the performance of this approach is
unworkable, we need orders of magnitude better"* (owner, verbatim, stage9
13.11). The belt is the largest thing this project has proposed
(docs/research/seam-temporal.md 4 refused it as the next thing on exactly that
ground). So its per-frame cost is measured here, on this box, against the
budget, with the floor device stated as a projection and labelled as one.

**How to read the evidence.** Every number carries a label. **Measured** means
this instrument produced it and the command is on record below. **Inference**
means it is a reading of measured things and could be wrong. **Projection**
means it is a measured number multiplied by a ratio, and the ratio's own
uncertainty is stated beside it.

---

## 0. The answer in one page

**On this box (RADV Phoenix, Radeon 760M), with the file playing, the belt fits
at the small strip sizes and does not at the large ones.** Measured, during real
playback of the owner's own May-01 flight through the app's own pass:

| strip | belt ms/frame, playing | of the 8.0-8.5 ms pass | of a 33.3 ms frame | fps held | dropped / 20 s |
| --- | ---: | ---: | ---: | ---: | ---: |
| none (control, 4 runs) | - | - | - | 29.87 to 29.92 | 1 to 2 |
| 2048 x 128 | **2.60** | 31% | 7.8% | 29.87 | 2 |
| 2816 x 128 | **3.43** | 42% | 10.3% | 29.87 | 2 |
| 4096 x 128 | **4.91** | 59% | 14.7% | 29.82 | 3 |
| 4096 x 256 | **9.47** | 115% | 28.4% | 29.77 | 4 |
| 8192 x 512 | **18.08** | 219% | 54.2% | 29.47 | 10 |

**Three findings decide the recommendation, and two of them were surprises.**

1. **The search is 70 to 95 percent of the belt.** Everything else -
   rectification, the pyramid, densification, the gate, the consuming lookup -
   adds up to 0.4 to 2.5 ms and is not where the decision lives.
2. **The GPU runs the belt at its floor clock while a film is playing.**
   Idle, this box boosts to 2600 MHz and the belt at 4096x256 costs 3.38 ms.
   Under a 30 fps paced player it sits at **800 MHz** and the same belt costs
   **9.47 ms**. Measured, at the clock sysfs, during both runs. **An idle
   benchmark of this design flatters it by up to three times**, and that is the
   single most important thing in this memo for anyone who measures a GPU
   feature on this project again.
3. **The strip sizes that fit the clock are the ones whose field is least
   trustworthy.** At 8192x512 the along-seam reading, which parallax cannot
   reach, comes back at +0.001 degrees; at 2048x128 it comes back at -0.652,
   and it is roughly constant in strip *pixels* across five candidate sizes
   rather than constant in degrees, which is the signature of an estimator bias
   and not of a seam.

**GO on Phoenix**, at 2048x128 to 4096x128, seeded from the previous frame,
every frame. **NO-GO on the floor device** at any size, at any cadence, by a
projection whose optimistic end is still 15.6 ms of a 33.3 ms frame for the
smallest strip - before the app's own pass, which has never been measured on
that part at all.

---

## 1. What was measured, and how

`kjerag-spike --bin belt`. Three modes, each on the owner's May-01 file, at the
`down1` view's own instant (63.5 s):

```sh
cargo run --release -p kjerag-spike --bin belt -- <file.insv> mode=geometry
cargo run --release -p kjerag-spike --bin belt -- <file.insv> mode=cost
scripts/quiet.sh ./target/release/belt <file.insv> \
  mode=live res=4096x256 seconds=20 hz=60 belt=seeded
```

**Every stage is timed by a timestamp pair the pass writes itself**, resolved
and read back once per frame. Nothing is inferred from a wall clock around a
submit; the repo already knows what that costs (`ScenePipeline::band_repeats`
exists because a redraw's wall time on a loaded box is the pass plus whatever
else ran). Nothing in the tree used timestamp queries before this branch, so the
probe opens `dmabuf::open_device`'s device with `TIMESTAMP_QUERY` added, which
is why that function is copied into the binary rather than called.

**The stages, and what is faithful in each.**

| stage | what the probe runs | faithful? |
| --- | --- | --- |
| 1 rectify | a render pass per lens into an `r8unorm` strip, reading a body-frame map and taking one bilinear sample | the map is exact: `Reframe::project` over the file's own calibration |
| 2 pyramid | two half-each-way passes per lens, four taps | yes |
| 3 search | inverse-compositional Lucas-Kanade, 8x8 patches at stride 3, one workgroup per patch, Hessian built once from the template and inverted once, one warped bilinear fetch per patch pixel per iteration, reduced in shared memory. No variational refinement | yes, and both workgroup shapes are measured (below) |
| 4 densify | residual-weighted mean of the 3x3 patch neighbourhood covering each pixel, into an `rg16float` field | yes, this is DIS's own densification |
| 5 gate | one confidence per direction round the ring, reduced over that column's patches | yes |
| 6 lookup | a 2560x1440 pass that samples both lenses, run with and without the field read, **and the difference is what is charged** | yes, and the control is why |

**The rectification map is a body-frame object and that is a design finding, not
a probe shortcut.** Its texel-to-source-pixel map depends on the calibration and
on nothing the view does, so it is built once when a file opens and read every
frame. Building it costs **0.04 s at 2048x128 and 0.69 s at 8192x512** on one
CPU thread (measured), which is inside the owner's two-second bar with room. The
term it leaves out is the rolling shutter, and at the seam an X4 reads the same
world direction down both lenses, so that term is worth 0.000 degrees there
(docs/research/insv-format.md 6.7). A camera where that is not true would have
to rebuild the map per frame and this memo does not price that.

### 1.1 The controls, which is why any of the field numbers mean anything

A search kernel that wanders and reports the wander looks exactly like a search
kernel that works. So the probe rectifies lens 0 a **second** time with the
across-seam angle displaced by a known amount, matches it against the unplanted
lens 0, and checks what comes back. Two plants, chosen to answer differently:

| plant | has to read | reads, 2048x128 | 4096x256 | 8192x512 |
| --- | --- | ---: | ---: | ---: |
| **+0.05 deg** | the plant, exactly | **-0.0006 deg error** | **+0.0010** | **+0.0003** |
| **+0.30 deg** | the plant, exactly | +0.134 deg error | +0.262 | +0.270 |
| either | 0 on the along axis | +0.001 px | -0.000 px | -0.000 px |

**The small plant passes to a thousandth of a degree at every size.** The kernel
is correct and it is sub-pixel accurate, and the along-seam axis reads zero, so
the two axes are not crossed.

**The large plant fails, and its failure is the measurement.** A cold search at
one level recovers **1.5 to 1.9 strip pixels** of a plant and no more, at every
candidate size (2048x128: 1.50 of 2.72; 5632x128: 1.89 of 2.72; 4096x256: 0.69
of 5.43; 8192x512: 1.08 of 10.86). That is a Lucas-Kanade patch's linearization
radius, it is the same in pixels whatever the resolution, and it says the
**coarse-to-fine ladder and the temporal seed are load-bearing rather than
optimizations**. Anything that displaces the field by more than about 1.7 strip
pixels between one frame and the next is outside what a seeded search can
recover.

> **This control also caught a bug in the probe itself before it caught
> anything about the belt.** The first version bound lens 1's picture to lens
> 0's planted coordinates, so the control strip was noise and the plant read
> +0.135 px where it had to read -2.716. Both plants read correctly once that
> was fixed. A control that cannot fail is not a control.

### 1.2 The search's shape, measured rather than assumed

The obvious mapping - 64 lanes, one patch pixel each - is **1.3 to 1.4 times
slower** than the same arithmetic in a workgroup of one RDNA wave (32 lanes, two
pixels each), because the barriers in the 64-lane version cross two waves.
Measured, cold ladder, idle: 4.41 against 3.06 ms at 2048x128; 9.62 against 7.15
at 4096x256; 39.36 against 29.09 at 8192x512. **Every number in this memo quotes
the faster of the two**, and the probe reports both so that the GO/NO-GO is not
decided on one layout's word.

The kernel is **not** at any hardware peak. At 4096x256, boosted, the finest
level runs 7.25 M bilinear samples per iteration in 0.606 ms, which is
**11.96 Gsample/s against this part's 83.2 Gtexel/s** and about 144 GFLOP/s
against 2.66 TFLOP/s. It is latency-, occupancy- and barrier-bound, which is
what makes the clock finding in section 3 as large as it is, and what decides
the scaling method in section 5.

---

## 2. The belt geometry, off the X4's real calibration

`mode=geometry`, from `CalibrationSet::from_insv` on the owner's own file, over
all 128 of the band's azimuths. Measured.

```
camera: Insta360 X4 Air fw v1.2.7_build1, delivered frame 3840x3840, 2 lenses
seam:   model overlap 14.436 deg, drawn handover 8.00 deg, baseline 33.35 mm
```

| the ring where both lenses have the picture | min | median | max |
| --- | ---: | ---: | ---: |
| width, degrees | **14.140** | 14.440 | 14.750 |
| source px per degree, along the ring | 31.40 | **31.44** | 31.56 |
| source px per degree, across the seam | 16.38 | **16.55** | 16.69 |

Walked out at 0.01 degrees through `Reframe::within`, with
`projection::CAP_MARGIN_DEG`'s whole degree taken back off: that test is
deliberately generous by half a degree per lens, and a strip built on the
generous figure would sample off the end of a fisheye circle at every azimuth.
The strip's across-seam span is therefore taken as the **narrowest azimuth's
14.140 degrees**.

**The seam is twice as finely sampled along the ring as across it, and that
decides the strip's shape.** 31.44 px/deg along against 16.55 across is the
fisheye's own compression near the rim: the seam great circle lands near the
image circle, where the tangential scale is the circumference and the radial
scale is squeezed. **At native source resolution the whole belt is 11319 x 234
pixels** - an aspect ratio of 48:1, not the 16:1 the candidate sizes have.

| strip | px/deg along | px/deg across | vs source, along | vs source, across | resident |
| --- | ---: | ---: | ---: | ---: | ---: |
| 2048 x 128 | 5.69 | 9.05 | 0.18 | 0.55 | 7.4 MB |
| 2816 x 128 | 7.82 | 9.05 | 0.25 | 0.55 | 10.2 MB |
| 4096 x 128 | 11.38 | 9.05 | 0.36 | 0.55 | 14.8 MB |
| 4096 x 256 | 11.38 | 18.10 | 0.36 | 1.09 | 29.7 MB |
| 5632 x 128 | 15.64 | 9.05 | 0.50 | 0.55 | 20.4 MB |
| 8192 x 512 | 22.76 | 36.21 | 0.72 | 2.19 | 119.3 MB |
| 11264 x 256 | 31.29 | 18.10 | 1.00 | 1.09 | 81.7 MB |

**The three candidate sizes in the brief all oversample across the seam relative
to along it, 8192x512 by 2.19 times, and they pay for it in the search**, whose
cost is the patch count and whose patch count is the texel count. 4096x128 holds
the same across-seam resolution as 2048x128 with twice the along resolution for
2.0 MB, and 8192x256 would hold 8192x512's along resolution for half its search.
Sizes are recommended on this in section 6.

### 2.1 The capture ranges, and a disagreement the memo has to carry

The design brief states the capture ranges as **+-1.5 degrees along** and
**+-0.7 degrees across**. The repo's own on-record search window is
`band::PERP_DEG` at **+-0.90 along** and `band::FAR_DEG..NEAR_DEG` at
**-1.2..+2.6 across** (`crates/render/src/band.rs:114-159`). These disagree on
both axes and in opposite directions, and this memo does not know which is
right. **The probe takes the union on each axis** - along +-1.5, across
-1.2..+2.6 - because that is the only reading that refuses nothing either
document asks for, and it is what the search is clamped to. In strip pixels:

| strip | along, +-1.5 deg | across, -1.2..+2.6 deg |
| --- | ---: | ---: |
| 2048 x 128 | +-8.5 px | -10.9 .. +23.5 px |
| 4096 x 128 | +-17.1 px | -10.9 .. +23.5 px |
| 4096 x 256 | +-17.1 px | -21.7 .. +47.1 px |
| 8192 x 512 | +-34.1 px | -43.5 .. +94.1 px |

**Every one of those ranges is far outside the 1.5 to 1.9 strip pixels a cold
search can capture** (1.1). The window is what the *ladder plus the seed* has to
cover between them, not what one search reaches.

---

## 3. The cost, idle and playing, and the gap between them

### 3.1 Idle, boosted: `mode=cost` on six real consecutive frames

Median frame, first discarded (no temporal hint, cold pipelines). Measured.

| stage | 2048x128 | 4096x256 | 8192x512 |
| --- | ---: | ---: | ---: |
| 1 rectify, 2 lenses | 0.172 | 0.339 | 1.109 |
| 2 pyramid, 2 levels x 2 | 0.042 | 0.096 | 0.246 |
| 3 search, **cold ladder** | 2.767 | 7.509 | 29.090 |
| 3 search, **seeded, finest level only** | **0.968** | **2.641** | **10.230** |
| 4 densify | 0.118 | 0.262 | 0.966 |
| 5 gate | 0.008 | 0.029 | 0.093 |
| 6 lookup, whole picture | 0.072 | 0.081 | 0.237 |
| 6 lookup, 8 degree corridor | 0.011 | 0.008 | 0.041 |
| **BELT, cold every frame** | **3.119** | **8.244** | **31.546** |
| **BELT, seeded every frame** | **1.320** | **3.376** | **12.685** |

Run to run these move by about 5 percent (three runs of 4096x128 read 4.77,
4.53 and 4.61 ms on the cold arm); the fields they produce are bit-identical
across those runs, so what varies is the clock and not the answer.

The lookup rows are the **difference** between two arms of one pass: the same
pass with no field read costs 0.30 to 0.34 ms, and that is what the shipped pass
already pays. The belt's own share of the consuming lookup is **0.008 to 0.041
ms over the 8 degree corridor** and is not a consideration at any size.

Per-iteration cost of the finest level, taken from the cold and seeded arms of
the same grid (8 iterations against 3): **0.235 ms at 2048x128, 0.621 at
4096x256, 2.390 at 8192x512**, over a fixed per-frame setup of 0.264, 0.777 and
3.059. A reader can price any iteration count from those two numbers.

### 3.2 Playing: `mode=live`, the same probe under the app's own pass

The player opens the file, decodes both 3840x3840 HEVC streams, runs the seam
band and the app's own projection pass into a 2560x1440 target, paces redraws by
due time on a 60 Hz display, and the belt is dispatched on the same queue. The
belt reads one fixed real frame while the player decodes real ones: **this arm
measures contention and the clock, not the answer** (3.1 measures the answer, on
real consecutive frames). One belt per frame and no controls, so the load is the
load a shipped belt would put there.

| arm | belt ms/frame | wall ms/redraw | fps presented | dropped / 20 s |
| --- | ---: | ---: | ---: | ---: |
| **belt off** (control, 4 runs) | - | 6.59 to 7.26 | 29.87 to 29.92 | 1 to 2 |
| 2048x128 seeded | 2.60 | 11.29 | 29.87 | 2 |
| 2816x128 seeded | 3.43 | 11.93 | 29.87 | 2 |
| 4096x128 seeded | 4.91 | 13.23 | 29.82 | 3 |
| 4096x256 seeded | 9.47 | 17.85 | 29.77 | 4 |
| 4096x256 cold every frame | 17.27 | 23.27 | 29.62 | 7 |
| 8192x512 seeded | 18.08 | 23.28 | 29.47 | 10 |

Every row is a 20 second run of the same binary at the same view on the same
file, and the belt column is the median of the run's own per-frame timestamp
sums (p90 and worst are on stdout: 4096x128 reads best 3.11, p90 4.94, worst
5.74).

The wall column carries two extra full-screen passes the probe runs as its
lookup control and a shipped belt would not, so it is **pessimistic by roughly
0.6 ms idle and more under load**; the belt column is not, because it charges
only the difference.

### 3.3 The clock, which is the finding

The belt costs **1.4 to 3.0 times more per frame while a film is playing than it
does idle**, and the ratio is not contention. It is the GPU's power state.
Sampled at `/sys/class/drm/card1/device/hwmon/*/freq1_input` during both runs:

| run | GPU clock | GPU busy | package |
| --- | ---: | ---: | ---: |
| `mode=cost`, belt back to back, nothing else | **2600 MHz** | 49-89% | 28-37 W |
| `mode=live`, 4096x256, film playing at 30 fps | **800 MHz** | 56-63% | 17-20 W |

800 is this part's bottom DPM state and 2600 its top; the span is 3.25x and the
measured inflation at 4096x256 is 2.97x. **A 30 fps player that does 10 ms of
work and then sleeps until the next frame is due is exactly the duty cycle that
keeps the governor at the floor**, and the package sits at 17 to 20 W against a
28 to 37 W ceiling, so it is not thermal and it is not a power limit. It is the
governor declining to boost for a load it reads as intermittent.

**The ratio shrinks as the belt grows**, which is the same effect from the other
side: **1.97x** at 2048x128, 2.67x at 4096x128, 2.80x at 4096x256, and **1.43x at
8192x512**, where the belt alone is 54 percent of the frame and the load stops
looking intermittent. Inference, well supported by the clock samples.

**What this means for anybody measuring this project again:** an offscreen
instrument that runs its pass back to back measures the boost clock, and the app
does not run at the boost clock. `--bin zoom` and `--bin playback` already
disagree for a related reason (docs/research/insv-format.md 6: 5.5 ms against
3.6, visible only under live decode). This is a second, larger instance of the
same trap, and it is worth its own line in AGENTS.md.

### 3.4 Half rate, which does not buy what it looks like it buys

Studio recomputes one frame in `frame_interval_of_flow_calc_` and holds between
(seam-temporal 2.4, measured, with the 2-or-3 ambiguity still open). Run here as
one belt in three:

| arm | belt ms on the frames it runs | wall ms/redraw, all redraws | fps | dropped / 20 s |
| --- | ---: | ---: | ---: | ---: |
| 4096x256 seeded, every frame | 9.46 | 17.88 | 29.77 | 4 |
| 4096x256 seeded, one in three | **9.48** | 10.80 | 29.72 | 5 |
| 4096x256 cold, every frame | 17.27 | 23.27 | 29.62 | 7 |
| 4096x256 cold, one in three | **24.70** | 15.42 | 29.67 | 6 |
| 8192x512 seeded, every frame | 18.08 | 23.28 | 29.47 | 10 |
| 8192x512 seeded, one in three | **36.70** | 18.78 | 29.45 | 10 |

**The frame that computes gets more expensive, not less**, by up to 2.1x, for
the reason in 3.3: a third of the duty cycle is a third of the reason to boost.
Half rate buys average wall time and average power; it buys **nothing** on the
worst frame, which is what a 30 fps target is actually about, and at 8192x512 it
makes the worst frame worse than a whole 33.3 ms budget. Measured, and it is the
opposite of what the fallback ladder assumed before it was run.

---

## 4. What the field looks like, and the honest doubt about it

`mode=cost` reads the finest level's answers back and reports them. Measured, on
the owner's May-01 flight at the `down1` instant.

| strip | px/deg along | patches under the gate | median **along** reading | median **across** reading |
| --- | ---: | ---: | ---: | ---: |
| 2048 x 128 | 5.69 | 81.3% | -3.708 px = **-0.652 deg** | +0.461 px = +0.051 deg |
| 2816 x 128 | 7.82 | 81.4% | -3.636 px = **-0.465 deg** | +0.320 px = +0.035 deg |
| 4096 x 128 | 11.38 | 81.1% | -4.542 px = **-0.399 deg** | +0.158 px = +0.018 deg |
| 4096 x 256 | 11.38 | 79.9% | -2.108 px = **-0.185 deg** | -0.655 px = -0.036 deg |
| 5632 x 128 | 15.64 | 80.6% | -3.440 px = **-0.220 deg** | -0.301 px = -0.033 deg |
| 11264 x 256 | 31.29 | 79.9% | -0.803 px = **-0.026 deg** | -0.667 px = -0.037 deg |
| 8192 x 512 | 22.76 | 80.6% | +0.025 px = **+0.001 deg** | -0.749 px = -0.021 deg |

**The along-seam column is the one to read, and it is a warning.** It sits
between -2.1 and -4.5 strip **pixels** at every size below the two largest,
across sizes whose along-seam sampling differs by 2.7 times. A physical angle is
constant in degrees; this is roughly constant in pixels. **Inference, and the
memo states it as one: that is an estimator bias from decimating the strip
below the source's own sampling, not a measurement of the seam.** At the two
largest strips - the only two at or above 0.72 of native along-seam sampling -
it collapses to -0.026 and +0.001 degrees.

**What stops this being a clean proof, and it has to be said.** The along-seam
disagreement is not zero in reality: `band::PERP_DEG`'s own doc records **0.17
to 0.67 degrees** of along-seam residual left by the best per-file fit across
three shooters and three camera models. So a reading of -0.2 to -0.4 degrees is
inside the range of a real effect, and this instrument cannot separate the two
on its own. What it can say is that the reading tracks strip pixels rather than
degrees, which a real effect would not. **And the same question lands on
`band.rs` itself**, whose correlation grid samples at 0.10 degrees, which is
0.32 of the source's own along-seam sampling - the same regime. That is a
question for whoever owns the band, raised here and not answered.

**The seeded arm agrees with the cold ladder but not exactly.** At 8192x512 the
seeded field's medians are -0.009 against +0.001 degrees along and -0.043
against -0.021 across, and it keeps 76.1 percent of patches against 80.6. Three
iterations from last frame's answer is close to eight from the ladder on this
content and it is not the same answer.

> **The seeded column has to start from the ladder and a refactor briefly stopped
> it doing so**, which is worth recording because the wrong version looked
> *better*: with the chain starting from a field of zeros the seeded arm read
> -0.013 degrees along at 2048x128 instead of -0.564, because three iterations
> cannot walk far from wherever they start and the chain then re-seeded itself
> from its own near-zero answer every frame. **A seeded estimator can be anchored
> to its own history rather than to the picture, and its output looks calm
> either way.** The probe now runs the ladder on the first frame and hands its
> answer on, which is what a real first frame does, and the numbers in this
> section are that version's.

**The open risk the seed carries, which this probe cannot close.** The belt is a
body-frame object, so when the camera turns, world content slides *along* the
strip. On the owner's own corpus the seam sweeps world content at up to
**21.8 deg/s** (studio-parity 1), which at 11.38 px/deg along is **8.3 strip
pixels per frame** - five times the 1.5 to 1.9 pixels a search can capture. At a
fixed strip texel the previous frame's answer is then a hint about different
content. It held on the six consecutive frames measured here; six frames of one
flight is not a claim, and a seeded belt has to be measured across a hard turn
before anyone believes the seeded column.

---

## 5. The floor device, as a projection and labelled as one

**Method, stated so it can be argued with.**

1. **Find the bound.** At Phoenix's boost clock the search reaches 14 percent of
   this part's texture peak and 5 percent of its FP32 peak (1.2). It is neither
   texture-bound nor ALU-bound; it is latency-, occupancy- and barrier-bound. A
   kernel in that regime scales with **aggregate throughput** - lanes times
   clock - rather than with any single peak.
2. **Take the throughput ratio.** Both parts, at their own top clock:

   | | Radeon 760M (Phoenix) | UHD 620 (Whiskey Lake) | ratio |
   | --- | ---: | ---: | ---: |
   | FP32 lanes | 8 CU x 64 = **512** | 24 EU x 8 = **192** | 2.67 |
   | top clock | **2600 MHz** | **1150 MHz** | 2.26 |
   | lanes x clock | 1 331 200 | 220 800 | **6.03** |
   | FP32 | 2.66 TFLOP/s | 442 GFLOP/s | **6.02** |
   | filtered texels/clk | 32 | 12 to 16 | |
   | texture rate | 83.2 Gtexel/s | 13.8 to 18.4 Gtexel/s | **4.5 to 6.0** |
   | memory bandwidth | 89.6 to 102.4 GB/s | 34.1 to 38.4 GB/s | **2.6 to 3.0** |

   At each part's *bottom* DPM state instead (800 against 300 MHz) the lanes x
   clock ratio is **7.11**.
3. **Bound it honestly.** The compute and texture ratios put the floor of the
   slowdown at **6x**, with 4.5x reachable only if Gen9.5's sampler is the
   16-texels-per-clock variant *and* the kernel becomes texture-bound there,
   which is the optimistic reading of a measurement that says it is not texture
   bound here. Nothing measured bounds it from **above**: this kernel uses
   shared memory and barriers heavily, Gen9.5's shared local memory and barrier
   behaviour is materially weaker than RDNA3's, and its 2.6x narrower memory
   feeds a decode this part is already close to its limit on. **The band taken
   is 6x to 9x, from the measured live figures.**

**The projection.** Phoenix live (section 3.2), times 6 to 9:

| strip, seeded every frame | Phoenix, playing | UHD 620 projection | of a 33.3 ms frame |
| --- | ---: | ---: | ---: |
| 2048 x 128 | 2.60 ms | **15.6 to 23.4 ms** | 47% to 70% |
| 2816 x 128 | 3.43 ms | **20.6 to 30.9 ms** | 62% to 93% |
| 4096 x 128 | 4.91 ms | **29.5 to 44.2 ms** | 89% to 133% |
| 4096 x 256 | 9.47 ms | **56.8 to 85.2 ms** | 171% to 256% |
| 8192 x 512 | 18.08 ms | **108 to 163 ms** | 326% to 488% |

**And the number that settles it is not the belt's.** The same scaling applied
to the app's *own* measured cost on this box - 6.59 to 7.26 ms per redraw at
2560x1440 with the film playing and no belt, over four runs - puts the shipped
pass alone at **40 to 65 ms on the floor device**, which misses 30 fps before a
belt exists. Either the
scaling is pessimistic, or the floor device does not hold 30 fps today at that
window size, and **this memo cannot tell which**.

**What would firm it up: the floor device itself, and nothing else.** Three
measurements on a real UHD 620, in this order, each cheap:

1. `--bin playback` with no belt, at a realistic window. Does the shipped pass
   hold 30 fps at all? If it does, the 6x-to-9x band is too pessimistic and
   every row above moves with it.
2. Whether that part can decode two 3840x3840 HEVC streams at 30 fps at all.
   Gen9.5's HEVC decode is specified around 4096x2304; two 3840x3840 frames at
   30 fps is roughly 885 Mpixel/s, which is well past one 4K60 stream. **The
   brief takes dual-4K decode as already running; that premise is itself
   unverified**, and if it fails the belt is not the problem.
3. `--bin belt mode=live` at 2048x128. One number, and it replaces the whole of
   this section.

---

## 6. The recommendation

**Strip: 4096 x 128.** Not one of the three in the brief, and the reasons are
measured.

- It holds **11.38 px/deg along and 9.05 across**, which is 0.36 and 0.55 of the
  source's own sampling, against 2048x128's 0.18 and 0.55.
- It costs **4.91 ms per frame while the film plays** - 59 percent of the 8.0 to
  8.5 ms pass, 15 percent of a 33.3 ms frame - and playback held 29.82 fps with
  3 dropped frames in 20 seconds against the control's 1 to 2.
- It is **14.8 MB resident**, against 29.7 for 4096x256 and 119.3 for 8192x512.
- It spends its pixels where the seam has them. 4096x256 costs 1.9x more for
  across-seam sampling the source cannot supply (1.09 of native), and the along
  axis - where the source has 31.44 px/deg to give - is the one the field's own
  bias tracks.

**If the field's along-seam bias (section 4) turns out to be real and
disqualifying, the next size up on that axis is 8192x256** - untested here, and
it should be the first thing the next round measures: it holds 8192x512's along
resolution for about half its search cost.

**Cadence: seeded from the previous frame, every frame, with the cold ladder on
the first frame of a file and after every seek.** The ladder costs 7.15 ms idle
at 4096x256 and 17.3 ms live, which is affordable once; running it every frame
is not, and the 0.30 degree plant (1.1) says skipping it on a cold start is not
an option either.

**What must be built with it, and this is not optional.** A seek and a hard turn
both invalidate the hint. The rule that falls out of the plant control: **when
the previous field is more than about 1.5 strip pixels wrong, the search will
not recover it, and nothing downstream will know.** The belt needs the same
shape of guard the band already has - a residual gate that refuses rather than
reports - and #171's walk-in guard is the measured precedent (right field 100
percent, wrong-sign 0 percent, twice-too-large 50 percent; stage9 12.3).

---

## 7. GO / NO-GO

### 7.1 Against "30 fps held on Phoenix with decode running": **GO**

Measured, during real playback of the owner's own file through the app's own
pass, at the recommended 4096x128 seeded every frame:

- **29.82 fps presented**, against the belt-off control's 29.87 to 29.92 over
  four runs.
- **3 dropped frames in 20 seconds**, against the control's 1 to 2.
- **0 starved**, **0 audio underruns**, sound within 0.0 ms of the picture, worst
  redraw 31.7 ms late against the control's 31.8.
- Belt cost **4.906 ms/frame** median over 596 frames, best 3.11, p90 4.94,
  worst 5.74.

Every strip from 2048x128 to 8192x512 held 30 fps on this box; the largest cost
10 dropped frames in 20 seconds and 29.47 fps, which is a real regression but
not a stall. **The clock is not the reason to refuse the belt on Phoenix.**

### 7.2 Against the floor device: **NO-GO**, on a projection, with its band stated

Projected 29.5 to 44.2 ms per frame at the recommended size on a UHD 620, on a
33.3 ms frame, before the app's own pass. Even the smallest strip's optimistic
end is 15.6 ms, which is 47 percent of the frame, on a part whose render pass
has never been measured. **No cadence rescues it**: section 3.4 measured that
running one frame in three makes the computing frame *more* expensive, not less,
and the projection scales that too.

**This is a projection and it is labelled as one.** It rests on a throughput
ratio, not on a run. The floor device would replace it with three commands.

### 7.3 The fallback ladder, with the measured numbers for each rung

Every rung measured on Phoenix during real playback. The floor-device column is
the section 5 projection, and the band is 6x to 9x.

| rung | what it is | Phoenix, playing | floor device, projected | verdict |
| --- | --- | ---: | ---: | --- |
| **0** | 4096x128, seeded, every frame | **4.91 ms**, 29.82 fps, 3 dropped | 29.5 to 44.2 ms | **the recommendation.** GO on Phoenix, NO-GO on the floor |
| **1** | 2816x128, seeded, every frame | 3.43 ms, 29.87 fps, 2 dropped | 20.6 to 30.9 ms | GO on Phoenix; the floor's optimistic end only |
| **2** | 2048x128, seeded, every frame | 2.60 ms, 29.87 fps, 2 dropped | 15.6 to 23.4 ms | GO on Phoenix, indistinguishable from the control. **The cheapest rung is also the one whose along-seam field is worst** (-0.65 deg, section 4) |
| **3** | 2048x128, seeded, one frame in three | not measured at this size | - | would buy average wall and **nothing on the worst frame** (3.4). Not recommended |
| **X** | 4096x256, seeded, every frame | 9.47 ms, 29.77 fps, 4 dropped | 56.8 to 85.2 ms | fits Phoenix, over the pass budget at 115 percent. NO-GO for the floor by a wide margin |
| **X** | 8192x512, seeded, every frame | 18.08 ms, 29.47 fps, **10 dropped** | 108 to 163 ms | refused: 54 percent of a frame and a real fps regression on the *development* box |
| **X** | any size, cold every frame | 17.27 ms at 4096x256 | - | refused: 2.9x the seeded arm for a field that differs by hundredths of a degree |

**The honest reading of that ladder.** There is no rung that is both cheap
enough for the floor device and good enough to be worth shipping. The rungs that
fit the floor's optimistic end are the ones whose field carries a several-pixel
along-seam bias, and the rungs whose field looks clean are 3 to 10 times over
the floor's whole frame. **A belt for the UHD 620 is not a smaller version of
this belt; it is a different design**, and the honest options are: ship the belt
as a Phoenix-class-and-above feature behind its own switch (which is what
docs/research/studio-parity.md 7 already says the belt is), or refuse it until
somebody measures the floor device.

---

## 8. What this memo does not claim

- **It does not claim the belt fixes the seam.** Nothing here looked at a
  picture. The field's accuracy is established only against a planted shift
  (+-0.001 degrees at 0.05 degrees of plant), not against the owner's eye or
  against any registry crossing.
- **It does not claim the seeded arm is safe.** Six consecutive frames of one
  flight, no turn, no seek. Section 4 states the risk with a number.
- **It does not price a per-frame rectification map.** The static body-frame map
  is right for the X4 and this memo says why; a camera that needs a live one is
  unpriced.
- **It does not price the field's own memory traffic against a real window.**
  The consuming lookup was charged as a difference over a 2560x1440 pass with a
  synthetic source map, which is the right shape and is not the shipped
  sampler.
- **It does not settle the +-1.5/+-0.7 against +-0.90/-1.2..+2.6 disagreement**
  (2.1). It takes the union and says so.
- **It has not been near the floor device.** Section 5 is a ratio.
