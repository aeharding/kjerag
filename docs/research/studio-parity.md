# Studio parity: the flat seam, and the line that is held

**The ruling** (owner, 2026-08-08, on his own footage, label-blind):

> Current architecture approved. You can merge the existing stitch with a wide
> band.

**What that architecture is**, in his own frame of reference - Insta360 Studio,
which is the picture he compares everything to:

| Studio | Kjerag |
| --- | --- |
| **Off** (no Optical Flow) | calibration + fusion + an anchored handover line. **This is what ships.** |
| **Optical Flow** | the belt: one displacement learned over the whole picture. Later, behind its own switch. |

**One mode at a time.** Studio does not blend the two: it either morphs or it
does not. Neither do we, and that is the decision this chapter records. What
shipped before 2026-08-08 was a third thing - a permanently-on, partially-
trusted, live-estimated morph inside a narrow corridor - and it is the thing
the owner had been refusing, one arm at a time, for four days.

---

## 1. The problem, in his words

The complaint that started it, 2026-08-05: the seam shimmers **"like refraction
through glass"** while a clip plays and is clean when the same frame is paused.

`~/kjerag-ab/discriminator1.sh` was built to split the two explanations - a
fixed residual distortion with content streaming through it, or the app's own
per-frame correction breathing under a still picture - because pausing cannot,
since pausing stops both. The answer came back on the second explanation, and
everything below follows from it.

There are two independent ways the app made a still picture move:

1. **The correction moved.** The corridor bend was re-estimated every frame from
   a live correlation. Where it was right it bought alignment; where it was
   wrong it drew the same content a slightly different shape on each frame.
   Widths breathed with it, because since stage 4 the fade's own width was a
   function of the same reading.
2. **The line moved.** Under a world-locked view the body turns and the view
   does not, so the 50/50 locus sweeps across world content - **up to 21.8
   degrees a second** on this corpus. Every static defect the seam has travels
   with it, and a defect that travels reads as a defect where the same defect
   standing still reads as the picture.

The owner's own theory, offered before either was measured, is why Studio's seam
is so much less visible with everything off: **Studio does not let the line
slide.**

---

## 2. The loop, arm by arm

Nine arms over one day, every one of them judged by eye on his own flights, most
of them label-blind. Binaries in `~/kjerag-ab/bin/`.

| arm | what it was | verdict |
| --- | --- | --- |
| `handover-demo` (3 arms) | the width, hot-swapped inside one playback at 3 / 8 / 12 degrees | "somewhere between 1 and 2" - between 3 and 8 |
| `ghost` | the corridor ramp replaced by one displacement of the back lens's whole picture, learned live | superseded before it was ruled on |
| `comb` | a dead neighbour cell no longer punching a hole through a live cell's correction | superseded |
| `flat` | **zero morphing**: the bend out of the render path, opacity-only crossfade | the direction he kept |
| `flat2` / `flat3` | frozen fade width; linear blend curve | kept |
| `flat4` | the line anchored on content, slewed to a new anchor when it ran out of room | refused: **"lurching"** |
| `flat5` | two lines held at once, dissolving between them, so nothing on screen ever travels | refused: **"every now and then it glitches. We need it to be smooth, that is a requirement. Perhaps we just need the seam to be smoothly transitioning instead, probably simpler too, with some fixing to prevent small movements when stopped at one position."** |
| `flat6` | one line, one closed-form law, no events | **approved**, and merged here |
| `v6` calibration | a re-derived calibration generation | **refused by his eye**; calibration stays v3 |

**He was right about which way was simpler, and that is the finding.** flat5 was
the more elaborate machine - two anchors, a dissolve, a promote, a retarget, a
clamp and a state - and it was rougher. A dissolve is an EVENT: it has a first
frame, a first frame is where a velocity changes, and a velocity that changes
inside one frame is exactly what the eye catches. flat6 deleted every event and
measured better on the instrument he was not looking at:

| over the July-14 fast segment, 900 redraws | flat5 | flat6 |
| --- | ---: | ---: |
| events (retargets / promotes) | 29 / 25 | 0 / 0 |
| worst frame-to-frame change in the drawn line's velocity | 59.2 deg/s | **11.3 deg/s** |
| the same, for the unanchored geometry it is drawn from | 14.2 deg/s | 14.2 deg/s |

flat5's line was four times rougher than the picture it was drawn from. flat6's
is smoother than it.

---

## 3. What ships

### 3.1 The seam is flat

Every lens is sampled at the ray itself. `Reframe::blend` runs the projection
and a scalar weight per lens, and nothing between.

The band still measures - the compute half is untouched, and the cells, the
along-seam fit, the confidences and the trust filter are all still produced
every frame. **What it measures reaches no pixel.** That is deliberate: those
readings are the evidence the belt will be seeded from, and an instrument that
stops measuring is an instrument that cannot tell you what the belt has to fix.

`band::tests::what_the_band_measures_reaches_no_sample` is that sentence as a
test: it asserts the reading is live AND that every landing is the projection of
the unbent ray.

### 3.2 The fade does not breathe

One width for the whole picture (`Reframe::handover_width`), 8 degrees, clamped
per file by the camera's own overlap. Since stage 4 the width had been a
function of the measured disparity, so it moved frame by frame with the near
field; a fade whose width breathes is one more thing at the seam that moves when
the picture does not.

### 3.3 The crossfade is the ramp

`BLEND_POWER` and `steepen` are gone on both twins. The exponent existed to
spend the handover over less picture so that a *bend* was less obvious inside
it; with no bend there is nothing for it to buy. The delivered 10-to-90 walk
goes back to 4.85 degrees of the 8 degree support (mean over 24 azimuths), which
is what the owner picked.

### 3.4 The line is held, and about 60 percent of the crawl goes

`SeamAnchor`, **on by default**. `KJERAG_ANCHOR=off` is a research escape that
puts the line back on the raw geometry; it is not a setting, nothing in the
window offers it, and it is read once.

**Read the heading, not the verb.** This chapter said "the line is held" and
that is not the whole truth, which a review found on 2026-08-09. The anchor
holds the **share's** 50/50 line exactly: `crossover` is a ramp in
`across_seam - shift`, so the whole share profile translates rigidly with the
offset and the arithmetic is exact about it. The picture draws the **weights'**
crossing, which is that share times each lens's own coverage depth,
renormalized (`claim`) - and the depths are fixed to the lenses and do not
translate with the line. So the drawn line moves by less than the offset the
anchor commands, and the shortfall is the camera's own overlap against the band
it hands over on.

| measured on the calibration fixture, mean over 24 azimuths | X4 Air | X2-class |
| --- | ---: | ---: |
| overlap / band, degrees | 14.44 / 8.00 | 9.18 / 8.00 |
| `overlap / (overlap + band)`, the linear-taper bound | 0.643 | 0.534 |
| **delivered per commanded degree** | **0.617** | **0.510** |
| the same, spread over the 24 azimuths | 0.610 to 0.624 | 0.499 to 0.522 |
| drawn offset at a 4.00 degree hold | 2.54 deg | 2.13 deg |
| drawn offset at the 3.55 degree rail the fast segment reaches | 2.25 deg | 1.89 deg |

**So the anchor reduces the seam's crawl by about 60 percent on the camera the
owner judged it on, and by about half on the narrowest one. It does not remove
it.** At `down1`'s worst the geometry sweeps 0.82 degrees of world content per
frame under the seam; unanchored the drawn line crawls by all of that, and
anchored it crawls by 0.31.

**The smoothness claim survives the correction, and was re-measured on the drawn
line to check it.** Frame to frame, the worst change in the drawn line's own
velocity is 4.78 deg/s at `down1` against a geometry floor of 12.22, and 9.67 on
the July-14 fast segment against 14.27. Both are below the picture they are
drawn from, which is the criterion; the commanded line reads 1.62 and 13.95 on
the same runs.

**flat6 behaves identically** - this is a property of the fusion the owner
approved, not of anything this merge changed - and he approved the picture by
eye against the unanchored one at the six registry views. It is recorded here
and in the PR's accepted tradeoffs rather than fixed, because fixing it means
moving the picture he chose.
`projection::tests::the_held_line_delivers_a_fraction_of_the_hold_it_commands`
pins the numbers, so the disclosure cannot drift away from the code.

Everything below about smoothness is unaffected and is about the same line: the
figures in this chapter of 0.0046 and 0.00050 degrees per frame are the
**commanded** line's motion on content, which is what the follow's own law
governs, and they are quoted as that from here on.

One offset, and one line of arithmetic:

```text
delta = target / (1 + POWER * gain * dt) ^ (1 / POWER)
gain  = RATE * (target / allowance) ^ POWER          RATE 10/s, POWER 10
```

`target` is what it would cost to leave the drawn line exactly where it is on
the content it is on, read back off one world direction through this redraw's
own pose. The law is the closed-form flow of `d(delta)/dt = -gain * delta`.

Four properties, and they are the four things he asked for:

- **No events.** No states, no dissolves, no promotes, no retargets, no
  thresholds, no branch that fires on one frame and not the next.
- **It cannot jump.** The flow is exact rather than a step of an integrator, so
  no size of `dt` overshoots: the divisor is at least 1, so the answer is always
  between the target and zero.
- **It is a length of FILM.** At `dt = 0` the law is exactly the identity, so a
  redraw with no new frame behind it moves nothing, and a run at 30 or 300 fps
  over the same film follows over the same picture. (This is also what lets an
  offscreen instrument at one draw per frame speak for a 60 Hz window - see §5.)
  **A redraw whose film runs BACKWARDS is a seek, and a seek anchors afresh**,
  which is the one event in the mechanism and sits on the one frame where every
  pixel already changed. Before 2026-08-09 a backward seek came out as a step of
  zero, which is the identity, so the line was pinned to a target read off a
  world direction from a different part of the flight and slammed to the rail on
  that frame.
- **Standing still costs nothing, with no knee to click on.** The gain is a
  single even power, smooth everywhere including at zero and in every
  derivative. At a quarter of the allowance it is one hundred-thousandth per
  second.

Measured at the owner's `down1` line over the ten seconds he named: the
**commanded** line moves at most **0.0046 deg per frame** while the unanchored
geometry sweeps at up to 21.8 deg/s, and his paramotor shakes the geometry
through 3.0 of the 4.0 degrees of allowance at a couple of hertz without any of
it reaching that line. What the picture draws is that line at the gain in
§3.4 - the drawn line still crawls, by about 40 percent of what it would crawl
unanchored.

**And the rail is the film's frame rate, which is a characteristic worth its own
paragraph** (found in review, 2026-08-09). The eleventh root above is the
continuum answer; the law is charged once per frame, the geometry's sweep
arrives as a jump and the leak is then read at a gain taken after that jump, so
the offset the law can hold at all has a closed-form ceiling:

```text
delta -> allowance / (POWER * RATE * dt)^(1 / POWER)
```

`POWER * RATE` is 100 per second, so the ceiling equals the allowance at exactly
100 fps:

| film fps | 24 | 30 | 60 | **100** | 120 | 240 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| ceiling, deg, of a 4.00 degree allowance | 3.47 | **3.55** | 3.80 | **4.00** | 4.07 | 4.37 |

Every file the owner has judged is 30 fps, where the ceiling is 3.55 and the
map's own clamp is unreachable - which is why the July-14 fast segment settles
at 3.53 to 3.55 and why `Reframe::with_shift`'s clamp never fires on it. Both
cameras in the corpus shoot **120 fps modes**, and there the rail is the clamp
rather than the law: the fade sits one-sided at its widest for as long as the
drift lasts. Making the law dt-invariant is a different picture at 30 fps as
well, so it is not a change this merge may make; it is documented, pinned by
`projection::tests::the_follow_has_a_ceiling_and_the_frame_rate_sets_it`, and
listed in §6.

The clamp firing is also why `SeamAnchor::hold` clamps **before** it places its
world anchor. It did not until 2026-08-09: the picture drew the clamped offset
while the state recorded the unclamped one, so every frame the clamp fired
re-made the same standing bias. At 30 fps the clamp cannot fire, so the fix
moves no byte of the arm the owner approved - and above 100 fps it is the
difference between a rail and a drift.

**One anchor for the whole ring**, not one per azimuth. The line elsewhere on
the seam circle still crawls. That is deliberate and it is a real limitation: a
per-azimuth offset is a field, and a field that varies along the seam is a warp
of the picture rather than a slide of a line. What the owner is looking at is
the piece of seam in front of him.

### 3.5 The width

8 degrees, unchanged, re-confirmed twice: the 2026-08-05 blind A/B ("2 is way
better", said of the 8 degree arm) and the 2026-08-08 hot-swap session. The knob
`KJERAG_HANDOVER_DEG` stays live, because it selects a value on a continuum the
next A/B will want to sweep again.

---

## 4. What died, and what governed it

| what | why it is gone |
| --- | --- |
| **the corridor bend** | `Reframe::bent`, `Reframe::bend`, `Bend`, `blend_bent`, WGSL `band_bend` / `band_rest` / `carry` / `table_at`. The thing the owner refused. |
| **the fold apparatus** | `FOLD`, `SPEND`, `WIDEST_DEG`, `band::carried`, `band::width`, `band::affordable`. Every one of them existed to keep the bend from printing the picture over itself. No bend, no shear, no inequality. |
| **the adaptive width** | stage 4. It solved the same fold inequality for the width instead of for the displacement. |
| **the steep blend curve** | `BLEND_POWER`, `steepen`. Half of #172. |
| **the ghost field** | the whole-picture displacement arm, staged 2026-08-08 and never ruled on. Not deleted for being wrong - superseded. It is the belt's own idea, and the belt is where it goes. |
| **the comb work** | a fix to how a dead cell zeroed a live neighbour's *correction*. There is no correction to punch a hole in. |
| **#172's temporal delivery** | the steeper curve is deleted outright. The filtered trust gate and the arrival staging are **not** deleted - they still shape what the band reports - but they no longer reach a picture, so what they now govern is the measurement an instrument reads and the belt will consume. |
| **the along-seam table's application** | stage 9. #164 had already refused the table on the evidence and no shipped run ever set one; the display path went with the bend. `Reframe::tabled` stays, because instruments read the raw planes through it to answer what is *still* wrong. |

**A live uniform member with no GPU consumer**, disclosed rather than fixed:
`Reframe::table` is still uploaded every redraw and no shader reads it. It stays
because this struct **is** the block - one definition, laid out once, checked
against WGSL by one test - and splitting it in two to save 512 bytes a redraw
would trade that invariant for nothing measurable.

---

## 5. The null, and where it cannot reach

**What he approved is what ships, and that is checked rather than asserted.**
`kjerag-spike --bin null` plays a real stretch of a real file offscreen, renders
every frame through the app's own pass, and prints an md5 per frame and one over
the run. The same instrument source is built against this branch and against
`5520259` - the commit `~/kjerag-ab/bin/flat6` was built from - and the two
summaries are compared.

Playing rather than seeking is the point: the band warms over seconds of film
and the line is held across frames, so a single seeked frame exercises neither.

**One `prepare` per frame is the app's behaviour and not an approximation of
it**, because the follow is charged in film and a redraw with no new frame
behind it is the identity (§3.4).

### 5.1 What cannot be identical, by construction

**The ONE X2's width, and this is the largest deliberate change in the merge.**
The bound on how wide a camera may hand over used to be its overlap *minus the
room a bend needed to carry a sample past the ray it was taken on*. Nothing
displaces a sample now, so an **unshifted** handover reaches half its own width
and no further, and the bound is the bare overlap.

> **The word "unshifted" is load-bearing and this chapter did not have it until
> 2026-08-09.** It read "the handover reaches half its own width and no
> further", stated as an invariant, and with the seam anchor on that is false.
> See 5.3.

Measured through the app itself on `VID_20251018_191318_00_002`:

| | flat6 | ship |
| --- | ---: | ---: |
| at open (factory calibration) | 4.91 deg | **8.00 deg** |
| after the per-file fit lands | 3.94 deg | **8.00 deg** |

The X2 overlaps by 9.19 degrees and the picture asks for 8, so it now draws the
whole ask with 0.60 a side to spare. Every X4 Air in the corpus already afforded
more than the ask and is unchanged at 8.00.

**So no X2 render can be byte-identical to flat6, and none is claimed.** The
owner's approval was given on X4 Air footage at the six registry views; the X2
change is a consequence of the architecture he approved, and it is a change
toward the width he chose rather than away from it.

### 5.2 The other place bytes may move

Deleting `steepen` is deleting a function that had become the identity at
`BLEND_POWER = 1.0` - but only *mathematically*. flat6 computed
`pow(s,1) / (pow(s,1) + pow(1-s,1))`, and on the GPU `pow(x, 1.0)` is commonly
`exp2(1.0 * log2(x))`, which is not exact; the denominator `s + (1-s)` is not
exactly 1 in f32 either. So the retired form and the deleted one can differ by
about one part in ten million of a weight.

That is far under one code of an 8-bit output almost everywhere, and "almost" is
not "everywhere": at a rounding boundary a pixel can flip by one code. The null
in §5 is what settles whether any did, and the result is reported in the PR
rather than predicted here.

### 5.3 The width bound is not the safety bound, and the anchor is why

**The claim this chapter shipped with, corrected.** 5.1 said the handover
reaches half its own width off the seam, and `Reframe::afforded` said the same
thing in a comment, and both offered it as the reason a band clamped to the
overlap cannot ask a lens for picture it does not have. **The support is centred
on the DRAWN line, not on the seam.** With the anchor's offset in it the ramp
closes at `-band / 2 - shift` and opens at `band / 2 - shift`, so off the seam it
reaches `band / 2 + |shift|`, and `|shift|` is allowed up to `band / 2`. At the
rail that is a **whole band**:

| | X4 Air fixture | X2-class |
| --- | ---: | ---: |
| shared picture, a side | 7.22 deg | 4.59 deg |
| the support at the rail | 8.00 deg | 8.00 deg |
| how far past the coverage | 0.78 deg | 3.41 deg |

**How often that happens on his own film**, read off the held line's own trace,
2026-08-09. `band / 2 + |delta|` against `overlap / 2`, frame by frame:

| | frames past the coverage | worst support | against |
| --- | ---: | ---: | ---: |
| `down1`, 10 s, X4 Air, overlap 14.56 | **0 of 300** | 7.13 deg | 7.28 a side |
| July-14 fast segment, 30 s, X4 Air, overlap 14.89 | **202 of 900** | 7.53 deg | 7.45 a side |
| the same offsets on an X2-class camera, overlap 9.19 | **866 of 900** | 7.53 deg | 4.59 a side |

So the roomy camera crosses barely and only under the hardest motion in the
corpus, and a narrow one crosses almost always. **The first commit of this round
said "91 percent of frames on a real X4 Air flight" and that figure is wrong**:
it is the X2-class number, taken from the review that raised the finding rather
than re-measured, and it was corrected here as soon as the trace was read. The
commit message is not rewritten, because this repository does not force-push. The two guards that were supposed to hold the invariant were
tautologies - one asserts `width / 2 < overlap / 2` for widths already clamped
to the overlap, the other reduces to `min(8, o) <= o` - and neither has a shift
in it.

**Nothing is narrowed, and that is deliberate.** Taking `|shift|` off the width
would make the fade breathe every time the line moved, which is the fault the
owner named directly (3.2), and it would change the picture he approved.

**What carries the overshoot is `claim`.** A lens's share is multiplied by its
own coverage depth, which reaches zero exactly where that lens runs out of
picture, so the outer lens fades to nothing on its own rim before the ramp can
ask it for a sample it does not have, and the pair renormalizes onto the lens
that does have the ray. Measured over the whole ring at every shift the clamp
allows, on both camera classes:

- **no hole**: the delivered weights sum to 1 at every direction, to a single
  ulp (0.99999988, which is `share`'s own division);
- **no cliff**: the worst step in the delivered weight is **0.0025** per
  hundredth of a degree on the X4 Air fixture and **0.0034** on an X2-class one,
  against a fade whose own mean slope over its delivered 10-to-90 walk is
  0.0017.

`projection::tests::the_anchored_handover_leaves_no_hole_and_no_cliff` is that
measurement, with two controls that make it able to fail: the coverage taper
broken to a hard edge steps the weight by 0.36, a hundred times the bar, and a
shift written past the clamp straight into the block opens a real hole. **The
guard against a hole is the clamp in `Reframe::with_shift`**, and the margin it
holds is `overlap / 2` - a hole needs `|shift| > band / 2 + overlap / 2` and the
clamp allows `band / 2`.

### 5.4 The twin guard, which is what the null was standing in for

Every number the map is made of exists twice: once as Rust the tests can reach,
once as the WGSL that same file emits, and they are kept in step by hand. On
2026-08-09 a review proved that nothing checked it - a bend planted inside the
WGSL `blend` alone, with the Rust twin untouched, left the entire workspace
green while the rendered picture changed. The null caught it, and the null needs
real footage, a GPU and a second build of the whole tree.

`crates/render/src/twin.rs` is the cheap version: it compiles the shipped
`projection::wgsl()` - the same string `Scene` hands wgpu, not a copy - with a
compute entry after it that calls `blend` on 5930 probe rays, and compares every
weight and every landing against `Reframe::blend`. Clean, on RADV Phoenix, the
two halves agree to **2.1e-6** of a weight and **9.8e-4** of a pixel; under the
review's own planted bend they disagree by **1.8e-3** of a weight, 887 times the
bar, and it is **the only test in the workspace that fails** (225 pass, 1 fails).

It needs a device, so CI skips it and `KJERAG_REQUIRE_GPU=1` turns that skip into
a failure; `scripts/uitest.sh` runs it that way, which puts it in the same seat
as the harness itself - the gate CI cannot run, run on the box that can, not
skippable on the way to a tag (release.toml).

**What it found on its first run, reported rather than fixed.** WGSL says a
`var` with no initializer is zeroed; on this driver it is not re-zeroed per
iteration of `blend`'s loop, so a ray only lens 0 has comes back with lens 0's
landing sitting in lens 1's slot. It reaches no pixel - `fs` samples each lens
behind `mix.weights[i] > 0.0` and that weight is exactly zero - so the test
compares landings where the weight is not zero and says why. Nothing in the
shipped shader is changed for it: changing the shader is changing the arm the
owner approved.

**What it does not cover**, said plainly: the fragment half - the NV12 sampling,
the colour transform, the write to the target - which needs decoded planes and
therefore real footage, and therefore the null.

---

## 6. Known characteristics, disclosed

These are accepted tradeoffs of the approved architecture, not defects with
fixes pending.

- **The honest doubling.** With no morphing, content the two lenses genuinely
  disagree about is drawn **twice** across the handover rather than smeared into
  one wrong shape. At the `bad` crossing view the epipolar misalignment now
  shows as visible doubling. The wide band is what softens it; the belt is its
  fix. The owner has seen this and chose it.
- **The anchor delivers about 60 percent of the hold it commands** (§3.4), so
  about 40 percent of the seam's crawl survives it on the X4 Air and about half
  on the narrowest camera. Measured, not estimated: 0.617 and 0.510 degrees
  drawn per degree commanded. flat6 behaves identically and the owner approved
  the picture by eye.
- **The one-sided fade truncation at sustained motion.** A sustained drift holds
  the line off-centre, so the fade is no longer symmetric about the geometric
  seam and one side of it is shorter than the other. The offset a drift of `w`
  can hold is `allowance * (w / (RATE * allowance))^(1/(POWER+1))`, an eleventh
  root, capped by the frame rate's own ceiling (§3.4): over the July-14 fast
  segment the line reaches 3.53 of its 4.00 degrees, so at worst the ramp's
  support runs 0.47 degrees on one side of the seam and 7.53 on the other, and
  the outer end of that is past the shared picture and carried by the coverage
  taper (§5.3). The owner was asked about this and did not object. **Recorded
  here as a known characteristic rather than treated as approval of it.**
- **The follow's rail depends on the film's frame rate** (§3.4), and passes the
  allowance above 100 fps. Both cameras in the corpus have 120 fps modes and no
  such file has been judged. Above that rate the clamp is what decides where the
  line sits, which is a fade held one-sided at its widest for as long as the
  drift lasts.
- **The ring crawls away from the view centre** (§3.4).
- **The far-field alignment the bend used to buy is gone**, and the instruments
  can still measure that it is gone. That is the trade he made: alignment that
  moves, for a seam that stands still.

---

## 7. Where the line goes next

The belt - Studio's "Optical Flow" column - is a displacement learned over the
whole picture rather than a ramp across a corridor, behind its own switch, one
mode at a time. Everything the band measures today is what it will be seeded
from, which is why none of the measurement was deleted with the application.

The parity line against the maker's own export continues separately; calibration
stays at v3 (v6 refused, 2026-08-08).
