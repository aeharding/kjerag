# The correction displaces, and the corridor carries only what is left

**Status:** a design memo and a perf envelope. **Nothing here is built.** One
probe is built and committed (`--bin ghost`), and every number below that is
labelled *measured* came off it or off the record it cites.
**Date:** 2026-08-08. **Branch:** `research/seam-ghost`, off `origin/main` at
965bf5d.
**Audience:** the owner, as the checkpoint before any of it is.
**Scope:** how to retire the corridor bend on the **across-seam** axis, which
is defect (2) of docs/research/seam-temporal.md 0 — the one his standing
complaint names and the one neither #172 nor #173 touched.

**How to read the evidence.** Every number carries one of three labels.
**Measured** means an instrument produced it and the run is on record.
**Inference** means it is a reading of measured things and could be wrong.
**Owner** means he said it.

---

## 0. The answer in one page

Today the band measures a per-direction across-seam disagreement and spends it
as a **ramp across the handover**: zero at the corridor edge, the whole
disagreement at the far edge. Where that disagreement is parallax the ramp is
right. Where it is the camera — which on the owner's May-01 downward arc is
**0.89 degrees, steady** (measured, this branch) — the ramp bends straight
lines and sweeps the bend along as the seam moves. That is his complaint,
verbatim: we *"migrate/warp the bad stitches together"* where Studio *"just
ghosts"*.

**The proposal is one change of shape and no new measurement.** Split what the
band already reads into the part that is the same every frame and the part that
is not:

- the **steady** part becomes a per-direction **displacement of lens 1's whole
  picture**, exactly the vehicle #169/#171 measured (BAD crossing 19.94 to 0.89
  view px, nine of nine crossings improved) and exactly the shape the
  **along-seam** channel already uses today;
- the **instantaneous** part stays in the corridor, ramped, because for real
  parallax a ramp is what a ramp is for;
- what neither can carry shows up as **soft doubling in the crossfade**, which
  is the principle.

The steady part is learned **live, from the readings the band already takes**,
by a servo whose fixed point is the band having nothing steady left to report.
No harvest. No offline field. No walk with abort criteria.

**What the probe measured, on his own clip, at his own banked view** (`--bin
ghost`, May-01, `down1`, `seam=pool`, the shipped pass's own `Cell` values read
back frame by frame):

| | |
| --- | ---: |
| arc directions with evidence at the **first** tick | **6 of 11** |
| the same after 0.5 s of playback | **9 of 11** |
| the field reaches 50% of its settled value at | **2.17 s** |
| 90% at | **5.61 s** |
| 99% at | **9.51 s** |
| what the corridor is left to ramp over the arc, today | **0.892 deg (45.7 view px)** |
| the same with the field | **0.026 deg (1.3 view px)** |
| what the servo costs, per frame | **8.5 us, 0.101% of the 8.44 ms pass** |

Against #171's **27.3 s to read the field and 135.1 s to walk it in**, which is
what the owner refused on the clock. This is the same mechanism with the
harvest deleted, because the harvest was never necessary: the band measures all
128 directions every frame while it draws.

**And the view he refused #171 on — the parking lot at `yaw=-146.13
pitch=-37.35` — reads the same defect.** The band's own settled reading over
his arc there is **-0.887 deg (-45.4 view px at fov 20)**, steady over fifteen
seconds, and the field takes the corridor's load there from 0.887 to **0.154
deg, 83%** (measured). #171 did nothing at that view because its harvest read
one of eleven cells there. Live accumulation reads ten of eleven within five
seconds.

---

## 1. What is being retired, and what is not

### 1.1 The one line that is the defect

`Reframe::blend_bent`, `projection.rs:874`:

```rust
let share = match lens { 0 => front, _ => 1.0 - front };
let carry = match lens { 0 => share - 1.0, _ => 1.0 - share };   // THE RAMP
let turn  = f32::from(u8::from(lens == 1));                      // THE DISPLACEMENT
let bent  = std::array::from_fn(|c| view_ray[c] + carry * bend.epi[c] + turn * bend.along[c]);
```

Both shapes are already in that one expression and the memo's whole proposal is
which of the two the across-seam axis uses.

- `carry` is the **ramp**. It is this lens's share of the disagreement, and it
  is the *other* lens's weight — zero where this lens is alone, half at the
  crossover, so the two lenses sit one whole disparity apart across the
  corridor. Content inside is stretched by the gradient. That is the warp.
- `turn` is the **displacement**. Lens 1 takes the whole thing everywhere, lens
  0 takes none of it. Nothing is stretched. **The along-seam channel has used
  this shape since stage 5 and nobody has ever complained about it.**

**The across-seam axis is the only channel still bending, and it is the one
carrying 0.89 degrees on his arc.** (Inference, from the two channels' own
sizes: the along-seam leftover is 0.094 to 0.119 deg rms and the across-seam
one is 0.42 rms on this flight.)

### 1.2 What does NOT die, stated before anything else

- **The corridor keeps ramping the instantaneous residual.** A near object
  genuinely subtends different rays from the two lenses and no single
  displacement can be right for two depths at once. That part stays exactly as
  it is.
- **The crossfade is untouched.** 8 degrees of support, `BLEND_POWER = 1.5`,
  the 3.85 degree delivered 10-90. That is where the ghost lives and #172 is
  the record of the owner choosing it blind.
- **The arrival staging is untouched** and the field borrows its filter class.
- **The along-seam harmonic is untouched.** Different axis, already displacing.

### 1.3 The tempting small change, and why it is refused

The smallest possible edit is to change `carry` to `turn` for the epi channel
and stop. It is about thirty lines and it is **wrong**, for a reason this
project has already paid for: the band's raw reading moves at video rate, and
applying a video-rate value to the whole picture is *more* picture moving
*faster* — which is the shape the owner's blind eye refused in PR #173
(*"a hole that does not move, against corrected picture that does"*).

**That refusal is the reason the servo exists.** The difference between the
field and the band's raw reading is not what it is worth, it is how fast it
moves. The field is slow by construction; the reading is not. Only the slow
part may go on the whole picture.

---

## 2. The design

### 2.1 The field: one number per direction, and it already has a type

**`band::Table`**: 128 entries packed 4 to a `vec4`, 512 bytes, indexed by the
ray's own cosine and sine with wrapping linear interpolation, riding in the
uniform block that `queue.write_buffer` writes **every redraw anyway**
(`projection.rs:534`, `scene.rs:1477`). It exists, it is tested, `Table::REST`
is exact identity, and #171 already put a second one beside the first
(`Reframe::epi`, `Reframe::with_epi`).

**So the field costs nothing to carry and nothing to upload.** The whole
per-session state is:

| | |
| --- | ---: |
| `field: [f32; 128]` | the estimate, radians |
| `seen:  [f32; 128]` | readings accepted, the gain schedule and the support count |
| `applied: [f32; 128]` | what the picture draws: smoothed, tapered, staged |

**1.5 kB per session, on the CPU.** Per camera and per file, not pooled — 10.12
is why (six flights disagree at a given azimuth by 0.597 deg at the median
against a pooled amplitude of 0.229 rms).

### 2.2 Where the displacement lands, and the taper that is geometry rather than a choice

Unchanged from #171 and its tests, which is most of why this is affordable:

```rust
let still = self.epi.at(body[0] / reach, body[1] / reach) * norm3(view_ray);
// ... and in blend_bent:
let bent = |c| view_ray[c] + carry * bend.epi[c] + turn * (bend.along[c] + bend.still[c]);
```

Three properties worth stating because each answers a question the brief asks:

1. **It cannot fold.** The displacement is across the seam and its gradient is
   along it, so the Jacobian it adds is off-diagonal and its determinant is
   exactly 1 (#171's
   `the_across_seam_term_displaces_lens_one_across_the_seam_and_nowhere_else`).
   This is why it needs no `SPEND`-style clamp of its own and why it does not
   interact with the fold inequality that cost #172 a review.
2. **The radial support is the whole hemisphere and the taper is not a
   parameter.** The term is a constant *vector* (`ACROSS_SEAM`, the body `z`)
   scaled by an azimuth-varying magnitude. A displacement `eps * z` applied to a
   unit ray turns it by `eps * sin(theta)` — **full at the seam, smoothly to
   zero at the back lens's own axis.** Nobody chose that; it is what a
   translation of a sphere looks like, and it is the same shape a real
   across-seam disagreement has. It also removes the singularity a per-direction
   displacement *direction* would have at the pole, where azimuth is undefined.
3. **The along-azimuth taper is `supported()`'s job** and is section 2.4.

**One honest cost, and it is new here.** The band measures along `Ring::epi`,
the epipolar line, which is up to **3.6 degrees round** from the seam normal
this term is applied along. Carrying a reading `d` on the normal therefore
leaves up to `d * tan(3.6 deg) = 6.3%` of it on the **along-seam** axis — on his
arc, 0.057 deg, which is over half of that axis's entire leftover. **It is
self-correcting and that is not luck**: the along-seam channel is *also* a
whole-picture displacement of lens 1, measured per frame, so it reads the
induced offset and takes it out. The memo does not hide it; it names it as
something the acceptance battery must show does not grow.

### 2.3 How it accumulates: a servo, not an estimator

The design decision that makes everything else small: **the field integrates
the residual, it does not estimate the truth.**

```
per readback tick, per direction with an accepted reading:
    seen   += 1
    r       = cell.disparity - applied              // what the band is STILL left to find
    gain    = max(1/seen, 1/256)
    lean    = 0.33 if r > 0 else 1.0                // the far gate, live
    field   = clamp(field + gain * lean * clamp(r, -0.25 deg, +0.25 deg), -2.8 deg, +2.8 deg)
```

Four things fall out of that, and each replaces a piece of machinery #171 had
to build:

- **The fixed point is `r = 0`** — the band reading nothing steady. That is the
  goal stated as arithmetic, and it means the field never has to know what
  "truth" is.
- **`|T - truth|` is directly observable.** stage9 12.3 is emphatic that the
  safety quantity is the residual and that *"nothing knows `truth` before the
  band has measured through the term"*. Here the residual **is** the servo's
  input. A field that is wrong produces a large `r` and is corrected by the next
  tick, in the direction that reduces it.
- **So the staged walk-in is subsumed, and this memo says so as a decision.**
  #171's four steps, its two abort criteria and its anchored direction set
  existed because a *whole field* arrived at once from an offline harvest and
  had to be validated before it was drawn. A servo never has a whole field to
  validate: it only ever moves by a step derived from a live in-window
  measurement, so it cannot walk itself outside the window it is measured in.
  **What survives from the walk is its control**, not its machinery — see 2.9.
- **`1/seen` is the seed and the settling in one line.** The first reading at a
  direction is the estimate; the hundredth is a hundredth of a correction. No
  separate seeding path, no "wait for k readings" rule.

**The `lean` is the far gate in its live form.** Parallax on this axis is
one-signed (`Cell::metres` is `reach_m / disparity` and exists only for a
positive disparity), so near content can only push a reading one way. Weighting
that way less settles the servo on a **low quantile** of what it sees rather
than on its mean — which is what `--bin epifield`'s offline "drop a moment whose
excursion implies nearer than 60 m" did, on evidence this never has to store.
**It is the same gate on the same axis, spent as a bias instead of as a corpus.**

### 2.4 Smoothness along azimuth, because the comb lesson binds

`field` is never drawn. What is drawn is `applied`, which is `field` put through
the **raised-cosine kernel and ridge `band::Table` already uses**
(`band.rs:857-891`: `smoothed`, `kernel`, `wrapped`), weighted by each direction's own support:

```
applied[i] = sum_j kernel(j - i) * support[j] * field[j] / (sum_j kernel(j - i) * support[j] + RIDGE)
```

- The kernel is zero at and past its own edge, so a direction walking into a
  window does not put a corner in the picture.
- `support[j] = min(seen[j], 8) / 8`, so a direction with nothing contributes
  nothing to either sum — **an unread arc is exact identity, by arithmetic and
  not by a branch**, and its neighbours taper into it.
- The ridge takes an entry with less than a reading's worth to nothing.

**This is what PR #173's refusal requires.** No cell walks on its own: every
direction's value is a weighted mean over roughly six degrees of azimuth either
side, so the field has **no teeth to comb**. The notch shape that got worse in
#172 and that #173 tried and failed to fix cannot form in a field of this
construction, because it is a shape that only exists where neighbouring cells
move independently.

**And the ridge is not free, which the probe priced** (measured, `down1`,
against the band's own -0.89 deg):

| `RIDGE` | the field settles at | 90% of it by |
| ---: | ---: | ---: |
| 1.0, the along-seam table's own | -0.754 deg | 7.21 s |
| 0.5 | -0.827 | 6.81 s |
| 0.25 | -0.866 | 6.41 s |

**The inherited 1.0 is a 17 percent haircut on a supported arc of this width.**
It should be chosen against this axis's own support and not copied; 0.5 is what
the envelope below is quoted at. Under-correcting is the safe direction — what
the field leaves, the corridor still carries, exactly as today.

### 2.5 The contract with the band, and why it is not a fight

**The band's bend shrinks as the field grows, structurally, and the record says
why.** The along-seam field failed on `T - fit(T)`: `Along::fit` is five terms
over the whole circle, so it reproduces `T` where the arc has evidence and
delivers `T` whole everywhere else. **The across-seam channel is not that
shape** — it is per cell, no fit, no ridge, no arc. Where a direction has
evidence the band reads the residual through whatever is applied and applies
that; where it has none it applies nothing (stage9 13.7, 10.9).

So the contract is:

> The field carries the part of the disagreement that does not change.
> The band re-measures through it and carries what is left, on the same axis,
> ramped, as it does today. Neither corrects what the other already did,
> because the band's input **is** the residual.

Measured precedents for the band not fighting it: with a per-session term
applied the band kept **96 of 128** directions with evidence (identical to off)
and its epipolar mean fell **0.554 to 0.190 deg** (stage9 11.4), and the
delivered picture was **steadier than `main` at every band**, with the one band
that stepped over a view pixel on 21 of 87 frame pairs falling to **0 of 87**
(stage9 12.5).

Modelled here on his own arc (the probe's one assumption, section 6):

| | today | with the field |
| --- | ---: | ---: |
| what the corridor ramps over his arc, `down1` | 0.892 deg | **0.026 deg** |
| the same, `down3` | 0.858 | **0.199** |
| the same, at the `#171` refusal view | 0.887 | **0.154** |

### 2.6 The rails

1. **`EPI_LIMIT_RAD`, 2.8 degrees**, on the applied value. stage9 12.3's number
   and its docstring's job: a rail against a field that is not a calibration at
   all. **It is not the guard** and the memo does not pretend otherwise. It
   differs from #171 in one way: #171 refused a whole field that broke it,
   because there was a whole field to refuse. A servo is clamped and **stops
   integrating at that direction, logged**, because there is nothing to refuse
   and a runaway is what the rail is for. Measured headroom: the largest value
   the probe ever applied was **1.10 deg** against the 2.8 rail.
2. **The step rail, 0.25 degrees per tick.** One correlation that found the
   wrong feature may not move the field further than the band's own filter would
   have.
3. **Composed identity.** #171's trap was that its term was `reading + the drawn
   pose's own 2.5 degree across-seam displacement`, so a direction with a zero
   *reading* still drew the whole pose arm and the composed term reached 4.141
   degrees. **That trap is gone by construction here**: the evidence is the
   band's live residual, measured through the map the picture is drawn with, so
   the term *is* the field and identity at an unread direction is `0`. The rule
   still binds and is still tested — identity is asserted on the composed term
   and not on the reading — but the composition is now trivial.
4. **The excursion-based far gate**, spent as `lean` (2.3). Not an absolute
   distance, which #171 measured to be nonsense on this axis: applied to the
   reading rather than the excursion it threw away 1829 moments of 3205 and
   called a calibration a hedge.
5. **`T - fit(T)` is structurally avoided** (2.5) and is not a rail anybody has
   to remember.

### 2.7 Arrival, seeks and restarts

- **Arrival.** `applied` is walked to its target through an exponential filter
  at `TAU_TRUST_S`'s own value, 2 s — the arrival staging's class, on a
  different quantity. This is why the field is drawn as a *fade in* and never as
  a step, and it is the second half of why no single reading can reach the
  picture: by the time the filter has walked a direction in, `seen` is ~60 and
  the estimate is settled.
- **Seeks: a no-op.** The field is a function of direction, not of position in
  the file. There is nothing to desynchronize. This is #171's property kept
  (*"a function of nothing that moves"*), for free rather than by design.
- **What a seek does change** is where the evidence comes from, which is the
  desired behaviour: #171's `Plan` had to spread its sample places across the
  duration deliberately, and playback does it for nothing. **A viewer who
  watches one minute gets a field for that minute** — disclosed in 2.9.
- **Restart / new file.** The field resets to the stored one for that file if
  there is one, else to the prior, else to `Table::REST`, which is `main`'s
  picture byte for byte.

### 2.8 Persistence, and what the pool may and may not carry

**The rule is simple and the evidence sets it: what pools is one harmonic; what
does not pool is stored per file.**

- **Per file, persisted.** The converged field, written on close beside the
  `seam_pool` entry in the same cosmic-config **state** (not config — a settings
  reset must not forget it, which is the pool's own rule). On reopen it is
  applied **whole at frame zero**, with no staging: at frame zero there is no
  previous picture for a step to be visible against. **This is the half of the
  design that meets the 2 second bar exactly**, and it is why "the player
  applies what it knows" is true here on every open after the first.
- **Not pooled per camera as a table.** stage9 10.12 refused that with a number
  and nothing has changed it.
- **Pooled per camera as one vector, if it survives a check.** The across-seam
  one-cycle term reads as one vector on five X4 flights (**0.2477 deg at 110.5
  deg**, chi2/dof 1.11 against a floor of 2.00, seam-temporal 2.5). It is a
  quarter of his defect and it costs nothing, and its value is that it shortens
  the arrival the first open cannot avoid. **It is increment 3 and not increment
  1, for a reason in 6.**

### 2.9 The honesty table

| when | what the picture is | who says so |
| --- | --- | --- |
| t = 0, first ever open of a file | factory + the pooled pose = **`main`'s picture, byte for byte**. `Table::REST` short-circuits and nothing is applied. | measured, #171's null 13.5 |
| t = 0 + 2 s, first open | ~50% of the field at the directions playback has already read | measured, 2.17 s at `down1` |
| t = 0 + 6 s, first open | ~90% | measured, 5.61 s |
| t = 0, every later open of that file | the stored field, whole, at frame zero | by construction, 2.8 |
| a direction the session never reads | **exact identity**, tapered in from its neighbours over the kernel | by arithmetic, 2.4 |
| an **arc** the session never reads | identity across the arc; the picture there is factory + pose, disclosed | by arithmetic |
| the field is wrong | the residual grows, the servo corrects it. Measured: a 0.5 deg wrong-sign field planted before any reading is **gone in 2.0 s** and costs about 3 s of extra arrival | measured, 3.4 |
| the field is very wrong | the 2.8 deg rail clamps it and stops integrating, logged | by construction |
| the pilot is shown a number | nothing. There is no toggle, no dialog and no progress bar | AGENTS.md's zero-config rule |

**Two cells on May-01, at 92.8 and 95.6 degrees, are never read in any run of
that file** (measured, seam-temporal 8.3). They are inside his arc. Under this
design they are drawn at identity and their neighbours taper into them across
roughly six degrees, which is the honest answer and is not a fix.

---

## 3. The perf envelope (rung 0)

**This is the gate that killed #171 and it is answered first.**

### 3.1 The probe

`crates/spike/src/bin/ghost.rs`, committed on this branch. It plays the file
through the **shipped pass**, reads the band state back frame by frame — the
very `Cell` values `ScenePipeline` dispatches into while it draws — and runs the
servo of 2.3, the smoothing of 2.4 and the staging of 2.7 on them, timing
itself. Box: **AMD Radeon 760M (RADV PHOENIX)**, the owner-box class, the same
one every number in stage9 13.9 was taken on.

**What is real and what is modelled, stated once.** The evidence stream is real.
One line is modelled: the band would read `disparity - applied` once the field
were applied, rather than the `disparity` it reads with nothing applied. That is
the linearised closed loop, and its warrant is measured elsewhere and not
re-derived — with a per-session across-seam term applied, the band kept every
direction it had and its epipolar mean fell 0.554 to 0.190 degrees. **So these
numbers are honest about coverage, about the value, about the wall clock and
about cost, and they are a model and not a measurement of the loop's stability.**

### 3.2 The two bars

**Frame rate.** Held, with three orders of magnitude of margin.

| | cost | against the 8.44 ms pass |
| --- | ---: | ---: |
| servo + smoothing + staging, per tick at 30 Hz | **8.5 us** mean, 28.5 p99 | |
| the same, amortised per frame | **8.5 us** | **0.101%** |
| the same at a 10 Hz readback | 5.8 us/tick, 1.9 us/frame | 0.023% |
| **applying** the field, per fragment | 2 uniform loads + a lerp + a scaled add | not resolvable |

The application number is not guessed: it is the *same instruction count* as the
along-seam `table_at` the fragment shader already runs, and PR #173 measured
going from two cell loads a fragment to **four** with presented frame rate
unchanged (29.80 to 30.04 fps on both arms). The field adds **no upload at all**
— 512 bytes inside a uniform block that is written every redraw anyway.

**The only new GPU work is a 4 kB buffer copy per frame** (126 kB/s) for the
readback. `ScenePipeline::band_state` already does the copy and the map; what it
also does is `poll(Wait)`, which is the stall no shipped path takes. The build
needs the non-blocking form: submit the copy, check last frame's callback, and
**skip the tick if it is not ready**. One to two frames of latency against a two
second filter is not a quantity.

*An engineering note that belongs in an envelope.* The probe's first cut swept
all 128 directions against all 128 in the smoothing and cost **98.7 us a tick**.
Restricted to the kernel's own support — nine taps, weights worked out once for
the ring — the identical arithmetic costs **5 to 9 us**. The 0.101% above is the
second one. The first would still have been 0.4% of the pass.

### 3.3 The clock

**This is the number the owner refused #171 on and it is the reason this design
exists.**

| | #171, refused | this design |
| --- | ---: | ---: |
| measured off the file before anything is drawn | **27.3 s** | **0** |
| walked in over | **135.1 s** | **5.6 s to 90%** (measured, `down1`) |
| total to a corrected seam, first open | **~162 s** | **~6 s** |
| the same, every later open of that file | ~162 s | **0 — frame zero** |
| the picture before it lands | `main`'s, byte for byte | `main`'s, byte for byte |

**About 27x on a first open, and the bar is met exactly on every open after
it.** The 2 second bar on a *first ever* open of a *new* file is not met and
this memo does not claim it: nothing that learns from footage can, and the
pooled prior of 2.8 is the only thing that can move that row.

### 3.4 The full arrival table, measured

`--bin ghost`, May-01, `seam=pool`, `rate=30 ridge=0.5`, arc 93 to 125 deg.

| view | arc read at tick 1 | at 0.5 s | 50% | 90% | 99% | settles at | ring, evidence |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `down1` | 6 of 11 | **9 of 11** | 2.17 s | **5.61 s** | 9.51 s | -0.974 deg | 71 of 128 |
| `down3` * | 5 of 11 | 6 of 11 | 2.30 s | 5.41 s | 7.41 s | -0.738 | 69 of 128 |
| the `#171` refusal view | 3 of 11 | 6 of 11 | 2.74 s | 10.88 s | 14.78 s | -0.697 | **98 of 128** |

\* `down3` was run at `rate=10 ridge=1.0`, so its settled value carries the 17%
ridge haircut of 2.4 and is not comparable to the other two on that column.

**The readback rate is load-bearing and that was not obvious.** Because the gain
schedule is `1/seen`, convergence is counted in ticks and not in seconds:

| readback rate | settles at | 90% by | cost per frame |
| ---: | ---: | ---: | ---: |
| 2 Hz | -0.505 deg | 8.51 s | 0.6 us |
| 10 Hz | -0.754 | 7.21 s | 1.9 us |
| **30 Hz, every frame** | **-0.894** | **6.01 s** | **9.0 us** |

**Read every frame.** It converges further and faster and it costs a tenth of a
percent of the pass. (Caveat, inference: at 30 Hz consecutive ticks are
correlated — `SLICES = 2`, so the band re-measures each direction every second
frame — so `seen` over-counts independent evidence and the gain schedule is
closer to an EMA than to a running mean. It converges anyway; the honest form
would tick on the band's own slice cadence.)

---

## 4. The increment plan

Each ends at an owner checkpoint. Line counts are estimates and are stated so he
can refuse one on size alone. Roughly 140 lines of what increment 1 needs is
already written and tested on `feat/per-session-epi` (#171) and is adaptation
rather than invention.

### Increment 1 — the live field. ~300 to 450 lines. **This is the build.**

The servo (2.3), the azimuth smoothing and taper (2.4), the non-blocking
readback, and #171's `Reframe::epi` / `Bend::still` / `ACROSS_SEAM` application
with its WGSL twin and its no-fold test. **No persistence, no prior, no
toggle.**

**Why this and not something smaller.** Two smaller things were considered and
refused on paper:

| smaller candidate | why not first |
| --- | --- |
| the pooled sinusoid alone (~40 lines) | 0.248 deg against his 0.906. His eye would be asked to rule on a quarter of the defect and would most likely answer "same", which decides nothing and costs a round |
| `carry` to `turn` with no servo (~30 lines) | applies a video-rate value to the whole picture, which is PR #173's refused shape at full size (1.3) |

**Its A/B arms, explicitly:**

- **A**: `origin/main` at 965bf5d — the bundle he picked blind on 2026-08-08,
  corridor bend intact.
- **B**: the same binary with the field on. Both arms one binary each, built in
  their own `CARGO_TARGET_DIR` (AGENTS.md, issue #47).
- **Views**: `down1`, `down2`, `down3`, `shimmer`, `good`, `bad` — the same six
  as the last two rounds, so the answers compose with them — **plus the `#171`
  refusal view** (`time=45.545 yaw=-146.13 pitch=-37.35 fov=20.00 lock=1`),
  which has never been in an A/B and is the view his last refusal was written
  about.
- **The briefing has to say two things**, because both are new percepts:
  *"straight lines crossing the seam should stop bending, and should stop
  dragging as the seam sweeps"*, and *"where it cannot be right you should see
  the same edge twice, faintly, instead of a distortion. That trade is the
  question."*
- **It also has to say the field fades in over about six seconds on a first
  open**, so that is not mistaken for the change.

**Gates before his eyes** (rung 0, all binding): the null — `Table::REST`
renders md5-identical to `origin/main` at all six views under `seam=pool` and
`seam=factory`, band live; the nine-crossing delivered battery with stage9
10.10's rule binding (**improve both May crossings without trading one for the
other**); `--bin band mode=snap` step counts not regressing (#173's own
acceptance, which #173 failed); the planted control of 3.4; and the along-seam
axis not growing by the 6.3% of 2.2.

**Rung 1, before/after stills.** The record has the owner refusing stills-first
for the *shimmer* work — *"live video is the only/best discriminator"* — and
that ruling was about a motion percept. **This defect has a static half**: a
bent horizon is visible in a paused frame, which is where `--bin epiramp`'s
whole nine-crossing battery lives. So stills come back as rung 1 here and cost
nothing, and the playback A/B remains the arbiter.

### Increment 2 — persistence. ~80 to 120 lines.

Write the converged field per file into cosmic-config state; read it at open and
apply it whole at frame zero. **Checkpoint: open the same clip twice and say
whether the second open is right when the frame appears.** This is the increment
that meets the 2 second bar, and it is deliberately second because it is worth
nothing until increment 1 has been judged worth having.

### Increment 3 — the pooled prior. ~40 to 60 lines.

The one-cycle across-seam vector as a per-camera seed, so a first open starts
part of the way in. Nine-crossing battery with the trade rule binding. **Its
sign must be checked first** (6, question 3).

### Increment 4 — the narrower blend, which is the owner's standing ask.

Named here as the follow-on and **not part of this design**. It becomes the
right question only once the corridor is carrying residual instead of
calibration: narrowing a crossfade that is spending a 0.89 degree camera error
would sharpen the error, and narrowing one that is spending 0.03 degrees is a
different change with a different answer.

---

## 5. What dies, and what stays

**Dies.**

- **The corridor bend as the carrier of the session's steady across-seam
  disagreement.** Stated precisely, because the loose statement is false: `carry
  * bend.epi` keeps carrying the instantaneous residual and stops carrying the
  part that is the same on every frame. Modelled size of what leaves, on his own
  arc: **0.892 to 0.026 degrees, 45.7 to 1.3 view px**.
- **The comb question on this axis, structurally.** A field that is
  kernel-smoothed over roughly six degrees of azimuth, ridge-tapered and moved
  through a two-second filter has no independent per-cell teeth, so the notch
  shape #172 deepened and #173 failed to fix cannot form in it. **This does not
  close the comb on the band's own per-frame channel**, where it remains open
  and unfixed.
- **#171's whole delivery**: the 27.3 s harvest, `Scene::learn_epi`, the
  four-step walk, its two abort criteria, its anchored direction set, and
  `--bin epifield` as a shipping path. What survives from it is the vehicle
  (~140 lines) and the control.
- **The composed-identity trap** as a live hazard — the 2.5 degree pose arm is
  not in the composition any more (2.6.3).

**Stays.**

- The along-seam harmonic, untouched.
- The blend at 8 degrees, `BLEND_POWER = 1.5`, untouched. It is where the ghost
  lives.
- The arrival staging, untouched, and its filter class reused.
- The band's per-frame epi ramp, for parallax, which is what it is for.
- `EPI_LIMIT_RAD` at 2.8 degrees, as a rail and not as a guard.
- The pool, and its refusal to carry a per-azimuth table.

---

## 6. Open questions, ranked

1. **Is the steady value at his arc the camera, or is it steadily-near ground?**
   This is the largest risk and it is the one #171 died on. The `lean` gate
   removes content that *wanders*; content that is near for the whole session
   sits in the middle and is learned. If his arc's -0.89 degrees is a parking lot
   at fixed altitude rather than the camera, the field displaces the whole
   hemisphere by a near object's disparity, which stage9 10.11 names as a worse
   defect than the shear it removes. **What is now known and was not**: the value
   reads -0.887 deg steady over fifteen seconds at the refusal view and -0.906 at
   the banked crossings, on content at three different places in the file — which
   is what a camera term looks like, and is also what a constant altitude looks
   like. **Cheapest discriminator**: the same probe run at two flights' worth of
   different altitudes over the same azimuths, or `Cell::metres` read out
   alongside. It is an afternoon and it should run before increment 1 is built.
2. **Closed-loop stability, measured rather than modelled.** Section 3.1's one
   assumption. Nothing in the record has ever run the loop. The probe cannot: it
   would need the field applied. **This is what increment 1's first instrument
   run answers**, and it is the one thing that could make increment 1 fail on its
   own gates rather than on his eye.
3. **The pooled sinusoid's sign at his arc, and its missing artifact.** Its peak
   is at **110.5 degrees**, which sits in the middle of his 93-to-125 arc, so
   whichever way it points it is not a small effect there. seam-temporal 2.5
   cites its table to `reference-views.md` and **that table is not in that file
   on `origin/main` or on `feat/comb-overlap`** (checked). Find the artifact and
   check the sign against the band's own disparity convention before increment 3
   seeds anything.
4. **`RIDGE`.** Inherited at 1.0 from the along-seam table and priced at a 17
   percent haircut here (2.4). Choose it against this axis's support.
5. **The 6.3 percent cross-axis leak** of 2.2. Argued to be self-correcting via
   the along-seam channel; not measured.
6. **The two cells at 92.8 and 95.6 degrees** that no run of May-01 has ever
   read, inside his arc (2.9).
7. **The ONE X2 has never been in an A/B**, and this is the third change in a row
   about which that is true (#172 9.6).
8. **Does the ghost read better than the bend to his eye at near content?** The
   principle's own test, and it is a question for the A/B and not for a number.
   Section 1 of seam-temporal is explicit that under this principle the near
   field looks like bounded doubling rather than bounded distortion, and that it
   is not obviously better to every eye.

---

## 7. The probe, and how to re-run it

```sh
# the arrival, the coverage and the cost at his banked view
cargo run --release -p kjerag-spike --bin ghost -- \
  ~/Videos/Insta/VID_20260501_183417_00_002.insv \
  from=65.666 count=450 yaw=179.00 pitch=-36.97 fov=20.00 lock=1 \
  seam=pool rate=30 ridge=0.5

# the control: a field that is wrong by half a degree before it reads anything
... plant=0.5

# the view the owner refused #171 on
... from=45.545 yaw=-146.13 pitch=-37.35 fov=20.00
```

Knobs: `rate=` the readback Hz, `smooth=` the kernel half-width in degrees,
`ridge=` the taper, `plant=` the control, `arc=low:high` the azimuths reported
on. Every run prints its own servo line, so a number quoted from it can say what
it was taken at.

---

## 8. Increment 1, as built (2026-08-08, `feat/ghost-field`)

**Status:** built, gated, and staged for a blind A/B. Nothing merged. Everything
in this section is **measured** on the delivered path unless it says otherwise;
the loop is closed and no line of it is modelled any more.

### 8.1 What is in the build

246 lines of shipped code, 262 of test, and a 445 line instrument.

| where | shipped code | what |
| --- | ---: | --- |
| `render/src/ghost.rs` | 96 | the servo, the azimuth smoothing, the taper, the far gate, the forgetting |
| `render/src/scene.rs` | 91 | the non-blocking readback, the tick, the env switch |
| `render/src/projection.rs` | 26 | #171's `Reframe::epi`, `Bend::still`, `ACROSS_SEAM`, `blend_bent` |
| `render/src/band.rs` | 31 | #171's WGSL twin and the compute pass's read-through, plus one shared unpacker |
| `render/src/lib.rs` | 2 | |

The vehicle is #171's, adapted rather than invented, exactly as 4 predicted. Its
no-fold test came with it (`seam.rs`, 48 lines) and passes unchanged.

`--bin ghost` is no longer a probe. It drives the shipped `Ghost` through the
shipped `ScenePipeline` with the field applied, so section 3.1's one modelled
line is deleted rather than improved on.

### 8.2 Three deviations from this memo, each forced by a measurement

**1. The staging filter is outside the servo loop.** 2.3's servo integrates
`disparity - applied`, which puts a two second lag inside an integrator. The loop
is then second order with a damping ratio of `0.5 * sqrt(TAU_MEMORY / TAU_TRUST)`,
so at the first reading's gain of 1 it is **0.065** and it rings for half a
minute. Measured before the fix, on a -0.9 degree truth: overshot to **-1.29**
and was still 0.29 degrees the wrong side of it four seconds later. The servo is
now told where the picture is *heading* - the smoothing's own output - rather
than where it has got to. The filter still walks the picture there, so 2.7's
property is kept whole and the gain schedule is free to be an estimator's again.
`the_servo_does_not_ring` is that failure kept as a test.

**This is the answer to 6's question 2**, and it is the one thing that could have
made increment 1 fail on its own gates. The loop is stable **once the filter is
taken out of it** and was not before.

**2. Forgetting is a floor under the gain, and the leak stops at one direction's
support.** The gain is `(1/seen).clamp(forget, 1)` where `forget` is
`ease(seconds, TAU_MEMORY_S)`, and `seen` leaks at the same constant but only
down to `SUPPORT_FULL`. Two reasons, both measured:

- a bound has to hold **whatever the value arrived with**, so it is the gain that
  is floored and not the evidence. With the leak on `seen` alone, the `seen=655`
  plant spent its first seconds throttled by its own false confidence.
- the leak may not take a direction's support to nothing. The first cut did, and
  at `down1` - whose arc goes dark seventeen seconds in - it gave the correction
  back for no reason but the dark, **-0.776 to -0.412 degrees**. #172 measured
  that a direction failing towards nothing is worse than one failing towards the
  reading it held, and that binds here one level out.

`TAU_MEMORY_S` is `2 * TAU_TRUST_S`, which is the relation rather than the
number: what is learned is always an average over more film than the walk it is
drawn through.

**3. `lean` is deleted; the far gate is the hard sign gate alone.** The
camera-vs-ground discriminator measured the blunt form strictly better on the
hazard and indistinguishable at cruise. Two gates on one axis where one is
measurably enough is a knob and not a design. The gate is asked about the
**whole** disagreement - the band's reading plus what the field draws - because a
distance is read off the whole and not off a remainder.

**And `RIDGE` is 0.5, chosen in this axis's units** (2.4, question 4). A direction
here carries eight readings of a cell that is already a two second average, so
`TABLE_RIDGE`'s "one reading's worth" is the wrong unit. It costs arrival rate
rather than value: the loop is closed, so the servo integrates until what is
*applied* matches what the band reads.

### 8.3 What the loop does, measured

`--bin ghost`, 900 frames, `seam=pool`, field on and off, arc 93 to 125 degrees
on the three downward views and the whole ring elsewhere. The corridor's load is
pooled over every frame past 12 s that read anything, weighted by how much of the
arc it read.

| view | corridor today | with the field | taken off | 50% | 90% | overshoot | turns |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `down1` | 0.9057 | **0.1728** | **80.9%** | 2.04 s | 5.64 s | +0.58% | 0 |
| `down3` | 0.9064 | **0.1608** | **82.3%** | 1.84 s | 5.51 s | +0.00% | 0 |
| the `#171` refusal view | 0.9124 | **0.1124** | **87.7%** | 4.27 s | 18.28 s | +0.50% | 0 |
| `shimmer` (July-14, whole ring) | 0.8445 | 0.9309 | **-10.2%** | 1.10 s | 2.17 s | +46% | 7 |
| ONE X2 (whole ring) | 0.1481 | 0.1205 | 18.6% | 1.40 s | 2.84 s | +22% | 15 |

**The arrival clock matches the probe's model almost exactly** (2.17 / 5.61
modelled, 2.04 / 5.64 measured at `down1`), which is the strongest thing that can
be said for 3.4 now that it can be checked.

**Two rows are honest about what they are.** `shimmer` and the X2 are whole-ring
means over views whose content changes while they play, so their "overshoot" and
"turns" are the scene moving and not the loop oscillating: `shimmer`'s mean walks
to -0.127 at 8 s and back to -0.088 as its covered set grows from 59 directions
to 118. The stability columns are only interpretable where the truth is steady.

**`shimmer` is a real worsened row and it is the design's own trade.** Past 12 s
its seam is looking at content 1.1 to 1.5 m away. The field there is small (-0.09
degrees mean) and the near disparity now sits **on top** of it, so the corridor
ramps 1.69 degrees where it used to ramp 1.45. The two lenses still agree at full
carry - the total correction is unchanged - but the corridor's gradient is
steeper by the size of the field. That cost is bounded by the field and it is the
principle stated as a number.

**2.5's contract holds and `T - fit(T)` did not happen.** The band's own reading
is what fell; it did not fight the term back.

### 8.4 The far gate, measured on the delivered path

Per direction over a whole run, a direction is called near-fed where the gate
refused most of what it was offered. The counterfactual needs no second servo: an
ungated `1/n` servo **is** a running mean of its input, so the mean of the whole
readings is what one would have settled on.

| view | refused | near-fed directions hold | with no gate they would hold |
| --- | ---: | ---: | ---: |
| `down1` | 4.2% | 0.147 | 0.541 |
| `down3` | 4.8% | 0.141 | 0.481 |
| the refusal view | 4.0% | 0.291 | 0.546 |
| `shimmer` | 87.0% | **0.070** | 0.512 |
| the landing, t=700 | 70.8% | **0.078** | 0.329 |
| the touchdown, t=770 | 34.4% | **0.068** | 0.697 |

A direction **every** reading of which is near learns exactly nothing, and that
one is a test (`a_reading_past_zero_cannot_move_the_field`) rather than a
measurement, because no stretch of real footage is that clean. What the live runs
show is the same thing with the mixture in it: near-fed directions hold two to
ten times less than they would with no gate, and what they do hold was learned in
the intervals when they were reading far.

The brief's landing chapter is at t = 700 to 776 of
`VID_20260501_183417_00_003.insv`, not t = 600; at 600 only 2 of 10 readings are
past zero and the aircraft is still up.

### 8.5 The plants

`--bin ghost plant= seen=`, whole ring, 30 s of real play at `down1`.

| control | at 0 s | 8 s | 12 s | 20 s | 30 s |
| --- | ---: | ---: | ---: | ---: | ---: |
| 0.500 deg at `seen=8` | 0.500 | 0.343 | 0.281 | 0.166 | **0.083** |
| 0.240 deg at `seen=655`, the poison | 0.240 | 0.177 | 0.136 | 0.038 | **-0.067** |

The poison passes through zero and goes on to learn the real field, which is
negative. Against the discriminator's measurement of the unbounded `1/n`
schedule - **still 0.119 degrees after thirty seconds** - that is the forgetting
bound doing its job.

**A hazard the probe could not see, disclosed.** A large wrong field **suppresses
the evidence that would correct it**: the band searches lens 1 through the term,
so a term far from the truth breaks the correlation. Measured at `down1` over the
whole ring, directions reading in the first seconds: **36 with no plant, 37 at a
0.10 degree plant, 32 at 0.25, 19 at 0.50**, recovering to 32 by 20 s. It is
self-limiting - the field walks out, the readings come back - and the servo
cannot reach a 0.5 degree error on its own, because every step it takes is at
most 0.25 degrees of a measured residual. But it is a real coupling and it is not
in section 2.

### 8.6 The gates

- **The null.** `--bin band mode=render count=40 size=1024`, band live, md5 of
  the rendered frame, at all six A/B views under `seam=pool` and again under
  `seam=factory`: **identical at all twelve** against a `main` binary built in
  its own target directory. The field-on arm differs at both fits at `down1`, so
  the comparison is not reading one binary twice.
- **Steadiness**, `--bin band mode=snap`, across-seam axis, the delivered step:

  | view | rms off | rms on | 3+ px off | on | 10+ px off | on | combed off | on |
  | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
  | `down1` | 0.49 | 0.82 | 15 | 26 | 1 | **6** | 40 | **38** |
  | `down3` | 0.78 | 1.05 | 32 | 38 | 2 | **6** | 100 | **36** |
  | `bad` | 0.30 | **0.23** | 7 | **2** | 0 | **0** | 256 | **40** |
  | the refusal view | 2.43 | **2.14** | 72 | **44** | 14 | **12** | 136 | **36** |

  **The two worsened rows are the arrival and nothing else.** Run at 900 frames
  instead of 300 the counts do not move at all - 26 and 6 at `down1`, 38 and 6 at
  `down3` - so every extra step is inside the first ten seconds and there are
  none after it. And `mode=snap` reads the **corridor's** channel only: the
  field rides in a different uniform, so what it reports as a step is the
  corridor giving up load that the field is taking on at the same instant.
  Disclosed rather than argued away.

  **The comb is much better and was not aimed at.** The hole #172 deepened and
  #173 failed to fix is a shape the corridor makes when it is spending most of a
  degree; spending 0.17 instead, it mostly stops making it.
- **Frame rate**, `--bin playback` 30 s, three reps with the order rotated, the
  two contaminated reps discarded and re-run: off 29.47 / 29.90 / 29.94, on
  29.94 / 29.94 / 29.90 fps presented. Not resolvable.
- **The servo's own cost**: 4.5 to 4.8 us a frame mean, 7 to 13 p99, which is
  **0.054 to 0.070 percent** of the 8.44 ms pass, against 3.2's estimate of
  0.101. No new upload; one 4 kB copy a frame, non-blocking.
- `cargo fmt --check`, `clippy --workspace --all-targets -D warnings`,
  `cargo test --workspace` (238 in `kjerag-render`, including nine on the servo),
  `scripts/name-check.sh`.

### 8.7 What is still open

1. **The corridor is left with 0.16 to 0.17 degrees at the downward views, not
   the 0.026 the probe modelled.** The probe compared a running mean against its
   own input, which is a measure of how well a mean tracks, not of what the
   corridor carries. 81 percent off is the honest number and 97 was never
   available.
2. **`shimmer` is worse by 10 percent on the corridor's load** (8.3), and the ONE
   X2 gains only 18.6 percent. Both are whole-ring statistics on views dominated
   by near content.
3. **The evidence-suppression coupling** of 8.5.
4. Questions 5, 6 and 7 of section 6 are untouched. The 6.3 percent cross-axis
   leak was not separately measured; the along-seam channel's own leftover is
   the thing that would show it.
5. **Persistence is increment 2 and there is none here**, so every open pays the
   six seconds. That is the row 3.3 says only increment 2 can move.
