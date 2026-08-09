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

### 3.4 The line is held

`SeamAnchor`, **on by default**. `KJERAG_ANCHOR=off` is a research escape that
puts the line back on the raw geometry; it is not a setting, nothing in the
window offers it, and it is read once.

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
  follows over the same picture. (This is also what lets an offscreen instrument
  at one draw per frame speak for a 60 Hz window - see §5.)
- **Standing still costs nothing, with no knee to click on.** The gain is a
  single even power, smooth everywhere including at zero and in every
  derivative. At a quarter of the allowance it is one hundred-thousandth per
  second.

Measured at the owner's `down1` line over the ten seconds he named: the drawn
line moves **at most 0.0046 deg per frame** while the unanchored geometry sweeps
at up to 21.8 deg/s, and his paramotor shakes the geometry through 3.0 of the
4.0 degrees of allowance at a couple of hertz without any of it reaching the
picture.

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
displaces a sample now, so the handover reaches half its own width and no
further, and the bound is the bare overlap.

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

---

## 6. Known characteristics, disclosed

These are accepted tradeoffs of the approved architecture, not defects with
fixes pending.

- **The honest doubling.** With no morphing, content the two lenses genuinely
  disagree about is drawn **twice** across the handover rather than smeared into
  one wrong shape. At the `bad` crossing view the epipolar misalignment now
  shows as visible doubling. The wide band is what softens it; the belt is its
  fix. The owner has seen this and chose it.
- **The one-sided fade truncation at sustained motion.** A sustained drift holds
  the line off-centre, so the fade is no longer symmetric about the geometric
  seam and one side of it is shorter than the other. The offset a drift of `w`
  can hold is `allowance * (w / (RATE * allowance))^(1/(POWER+1))`, an eleventh
  root: over the July-14 fast segment the line reaches 3.53 of its 4.00 degrees,
  so at worst the fade runs 0.47 degrees on one side and 7.53 on the other. The
  owner was asked about this and did not object. **Recorded here as a known
  characteristic rather than treated as approval of it.**
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
