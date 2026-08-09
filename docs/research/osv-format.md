# The DJI Osmo 360 `.OSV` format, and what it takes to reframe it

The second camera. Written 2026-08-09 against the branch that ships it, from
seven captures of two units: six from the owner's own camera ("unit B") and one
sample from another ("unit A"). Everything here was derived from those files;
there is no published specification, no `.proto` anywhere, and nothing was
transcribed from another project.

The `.insv` chapter beside this one is the reference for the other camera and
for everything the two share, which is most of the render path. This chapter is
only what is different, plus the honest list of what is still not known.

## Confidence key

The same one `insv-format.md` uses.

- **HIGH**: verified against both units, or against the picture.
- **MEDIUM**: consistent across every file read, with a mechanism.
- **LOW**: a reading that works, with no independent confirmation.
- **UNKNOWN**: named here so the next reader does not think it was checked.

## 1. Container, and where the calibration lives

**HIGH.** An `.OSV` is a plain MP4. The video is two HEVC streams, one per
lens, delivered as 3840x3840 squares. What is not plain is where the
calibration is: an `.insv` keeps it in a trailer after the last box, and an
`.OSV` keeps it **in the file's own telemetry track**, as a media sample like
any other.

The track is a `trak` whose `stsd` entry is `djmd`. An `.OSV` writes **two** of
them, and only the first carries the calibration: the second one's samples are
about a sixth the size and hold neither the calibration nor an orientation.
`kjerag_meta::osmo::telemetry_track` takes the first and says so.

Sniffing is by that same box (`(Parent::Stsd, b"djmd" | b"dbgi")`), with the
`.osv` extension as the fallback, so a renamed file is still recognised and an
`.OSV` that is really something else is still refused.

**The `djmd` box says DJI and not which DJI**, and every camera of theirs writes
one. What names the camera is `©too` at `moov/udta/meta/ilst`, which reads
exactly `Osmo 360` on all seven captures of both units, with no firmware version
after it. `kjerag_meta::format` compares it whole, so a Pocket, an Action or a
drone keeps the named "That is a DJI video" refusal instead of being taken into
a protobuf walk over a schema that is not its own. A firmware that starts
writing something else goes the same way, which is the fail-closed direction.

There is also a nested MP4 in a `camd` box. Its `mdat` is a byte-identical copy
of the outer `djmd` tracks, so it is read past.

## 2. The record: a protobuf with no schema

**MEDIUM**, and it is the load-bearing uncertainty of this whole chapter. Each
telemetry sample is a protobuf message. No `.proto` for it exists publicly, so
`kjerag_meta::osmo` walks the wire format by hand and reads **by field number**.
Everything it does not recognise is walked past, which is what makes the reader
survive a firmware that adds fields.

The field numbers it knows are all in one place, `osmo::field`, and that module
is the whole of the schema Kjerag claims. The two that matter most:

- `ORIENTATION = 28` on the calibration record: the per-lens entry.
- `FRAME = 3` / `STATE = 2` on every record, holding `POINTING = 9` (the fused
  orientation, four `f32`, `w` first) and `ACCELEROMETER = 10` (three `f32`, in
  g).

The parsed intrinsics match an independent scoping pass's table exactly, to the
last digit of the `f32`, on both units. That is the strongest evidence the field
numbering is right.

## 3. `djmd`: the lens calibration

**HIGH** for the numbers, **MEDIUM** for the meaning of two of them.

Per lens: `fx`, `fy`, `cx`, `cy`, and five distortion coefficients. There is no
mirror parameter, because this is not a Mei model (section 4).

### 3.1 The fifth coefficient is at field 15, seven fields away

**HIGH, and it is the headline of this chapter.** Four of the five coefficients
sit together at fields 5 to 8. The fifth is at **field 15**, past the
yaw/pitch/roll triple, with no marker to say it belongs to the run of four.

`K: [u32; 5] = [5, 6, 7, 8, 15]`.

Reading only the run of four is not a small error and it does not look like one:
four coefficients turn the radius over between 88.4 and 89.8 degrees off axis
and bring it back down, so the model folds, and a fold lands a ray from behind
the lens on a pixel belonging to one in front of it. With four, a 199 degree
lens reads as a sub-hemisphere one. **With the fifth the radius climbs
monotonically to half a turn on all four lenses of both units.**

What it was worth in the picture: the two lenses drew the same far content
**241 source px apart across the seam on the owner's camera, 13.1 degrees**, and
**3.2 px, 0.18 degrees** after. That is the seam tear the owner reported as the
picture being "fixed off everywhere" with a doubled tower, and his verdict on
the fix was *"can confirm tear is gone"*.

The equidistant map that shipped before the coefficients were read at all is the
control that says the test can fail: it answers 209 to 211 degrees of coverage,
a lens nobody makes.

### 3.2 The image circle is the frame, not the inscribed circle

**HIGH.** The Osmo delivers its image circle inscribed in the 3840 px square:
the picture reaches all four edges at the mid-sides and the corners are
unexposed. So the circle's radius is half the frame's shorter side and its
centre is the **frame's**, not the principal point's.

Taking the inscribed circle about the principal point, which is right for an
`.insv`, is wrong in both directions here at once: the number moves 1910.2 to
1916.7 px across four lenses purely because the principal point wanders 10 px
about the frame centre, which is a property of nobody's optics, and every one of
those frames is lit past it.

### 3.3 The polyline at fields 22/23 is a model constant, not a rim

**HIGH.** Each lens entry carries a fourteen-point polyline. It is
**byte-identical across all four lenses of both units**, so it cannot be a
per-unit measurement; it is a constant of the model. It looks like where the
camera body cuts the picture and would be worth applying as a body mask. Kjerag
reads it and applies nothing.

## 4. The projection model: Kannala-Brandt, not Mei

**HIGH.** The `.insv` path is Mei/UCM with Brown-Conrady distortion on the
normalized plane. The `.OSV` path is the Kannala-Brandt fisheye: the angle off
the axis onto a radius, through an odd polynomial in that angle.

```text
r = fx * theta * (1 + k1 t^2 + k2 t^4 + k3 t^6 + k4 t^8 + k5 t^10)
```

with `t = theta` in radians and `fy / fx` squeezing one axis against the other.
That is the whole model.

Both families are one function of a unit ray in the lens's own frame and nothing
else, so the model is a branch inside `lens_pixel` and the rest of the pass -
the caps, the crossover, the handover, the readout - does not know which one it
is running. Both take five coefficients, which is why `LensBlock` carries five
slots and not ten, and why neither model reads a slot by any name but its own.

**Where it may be believed.** A polynomial is a fit and not a law, so it is not
monotone by construction the way `r = fx * theta` is. The same question Mei
answers with its mirror parameter's turning point is answered here by the
derivative, computed as one Horner chain beside the radius and exact at the ray.
On this camera family it never fires; on a calibration that folds it stops the
picture where the fold is.

**One direction only.** Nothing in the render crate inverts either model. The
pass is a backward map - an output pixel becomes a ray and the ray is projected
into each lens - so the forward direction is the one the sampling asks for. The
inverse exists in the tests, by Newton, and is used only to check that the
forward map is a map at all.

## 5. Colour: 10-bit, studio swing

**HIGH.** An `.OSV` decodes to P010 (10-bit, two bytes a sample) in **studio
swing**, where every `.insv` in the corpus is 8-bit NV12 full range. Both had to
become one shader.

Two things were wrong at first and only one of them was obvious:

1. The 16-bit planes are imported two bytes at a time, because the device cannot
   make a 16-bit normalized texture, and the shader puts the word back together
   itself. P010 keeps its ten bits at the **top** of the word with the low six
   zero, which is why the scale is 65472 and not 65535.
2. **Both rows of the level table carry a whole excursion, not half of one.** A
   studio-swing chroma plane runs 16 to 240 at eight bits, which is 224 codes end
   to end, and 224 is what takes it to the -1/2..+1/2 the colour matrix is
   written for. Written as 112 this doubled every colour a DJI capture had and
   left every Insta360 one alone, because only studio swing comes through that
   path. Greens went neon and the owner's eye caught it on the first `.OSV`
   played.

Measured on unit B frame 0 over 451 flat patches against swscale's own decode of
the same frame: mean absolute error per channel fell from 45.4 / 8.0 / 0.9 codes
to **2.0 / 1.8 / 0.9**, and the mean chroma spread from 171.4 to 124.1 against
swscale's 125.1.

**Both facts are read off the container and neither is guessed from the other**
(`kjerag_media`'s `reader::written`). Two edges are worth naming:

- **Big endian is refused by name, not drawn.** The shader reassembles a 16-bit
  word from two 8-bit components in one order, so a big-endian stream would come
  out as noise with nothing to say so. Nothing in this path can produce one -
  ffmpeg names the host's endianness on a decode - so the refusal is a claim
  declined rather than a case anyone has met.
- **An untagged range reads as studio swing.** `H.264` and `HEVC` both default
  `video_full_range_flag` to 0, so a file that says nothing is saying studio
  swing. Measured over the whole sample corpus 2026-08-09: every Insta360
  capture, proxy and GoPro file is `yuvj420p` tagged `pc`, and every `.OSV` of
  both units is `yuv420p10le` tagged `tv`. **Nothing in the corpus is untagged**,
  so this fallback picks nothing that ships today.

## 6. Orientation: the camera solves its own, in a left-handed frame

**HIGH** for the composition, **MEDIUM** for the turn.

The file carries a fused orientation per frame - the camera did the work, so
there is no raw gyro track, no axis convention to name and no filter to run. The
whole composition is:

```text
world_from_body = BODY^-1 . mirror_y(w, x, y, z) . BODY . Rot(up, +90 deg)
```

Three things had to be found, and each was found by being wrong first.

- **The quaternion is written the other way round.** What the file writes takes
  a direction from the world to the body; Kjerag's `world_from_body` takes one
  the other way. Read as written, the picture turns twice as far as the wearer
  instead of standing still, which is what the owner reported as "the camera
  rotates when the wearer turns around".
- **The change of basis.** The file's world is `z` up and Kjerag's is `y` down.
- **The mounting, and this is the part that was wrong twice.** With the
  conjugate reading the heading was right and the tilt was not: through a dip
  the lock left nearly twice the tilt that switching it off left. The lean
  MAGNITUDE was right to 0.18 degrees and the direction that lean pointed, round
  the camera's own vertical, was wrong by about 135. The instrument that
  separates the candidates is the vertical vanishing point of a **lock off**
  render, which measures the world's up from the picture alone: each instant
  states one turn on its own, and the SCATTER of those over instants that lean
  in different directions is what tells the families apart, because only the
  right family leaves a constant of the hardware behind.

Over 23 instants of three unit B files and 177 degrees of lean azimuth:

```text
family                  turn      rms scatter    worst    walks with
as written             -84.6           65.9      148.8    heading 0.69
conjugate (pre-08-08) -107.1           62.1      133.2    azimuth 0.73
mirror in y            +86.8            3.3       10.1    nothing 0.15
mirror x, conjugated   -90.9           61.2      170.8    azimuth 0.89
```

**One of the four is a constant and the other three are not.** So the file's
inertial frame is **left handed** against the optical one, and that is also why
the heading looked settled while the tilt was not: a mirror reverses the heading
exactly as a conjugate does, so a turn measurement cannot tell the two apart.

The shipped turn is `+90.0` and not the measured `+86.8`. The two are 3.2
degrees apart, inside the measurement's own 3.3 rms scatter, so the corpus
cannot separate them; a right angle is what a screw can hold, an estimator
returns `+86.8`, and the quarter turn puts the mirror plane on the 45 degree
diagonal between the two lenses. `+90.0` is the arm the owner tested and
approved.

**This table is the whole of the mounting's verification**, and it is offline
and after the fact: rendered pictures, three files of one camera, one session.
Nothing at open re-checks it, and section 6.2 is the proof that nothing at open
can.

### 6.1 The self-check: what it verifies, and what it cannot

**HIGH**, and the second half of this section is a correction made in review on
2026-08-09. The check exists and does real work; the claim printed over it was
bigger than the work.

The accelerometer at field 10 is a second, independent statement of where down
is, in the same inertial frame as the quaternion, so `osmo::Plumb` holds the two
against each other per file and refuses the lock on a capture whose own two
records contradict each other.

It is read on frames leaning more than 8 degrees, because every reading predicts
the same lean magnitude - `1 - 2(x^2 + y^2)` carries no sign - so the readings
differ only in azimuth and only in proportion to the lean. The accelerometer is
low-passed over 2 s first, because a worn camera's accelerometer is gravity plus
the wearer's stride and on this corpus the stride is the bigger of the two at
frame rate.

Measured over the whole seven-file corpus, 2026-08-09, in degrees at the median,
by the shipped check itself (printed to four decimals for this table and to one
in the app's own line):

```text
file                                    shipped   flipped   null   leaned frames
1- 8k30p stable                          3.7510   10.9866  5.6774            39
1 8k30p standard 10bit -003 (owner's)    1.9534    9.2554  9.4379           539
2 8k30p Dlog-M -002                      2.8710   10.1090  6.8389           244
2- shake_stable_shake                    2.8713    5.3565  8.2701           680
2 Lens Sharpness Test                    1.5544   15.5463  7.6547            72
3 8k50p Dlog-M -001                      2.6931   11.3220  8.8967           893
CAM_20250715191201_0003_D  (unit A)     23.0196   11.3151 14.7221            53
```

Three bars, each with a job: the miss must be under **8 degrees**, which bounds
what a held horizon can be wrong by; under the **null** of a camera assumed
never to lean, which is what makes this a verification rather than a tolerance;
and under the **flipped** reading of the same quaternion, which is the one bar a
mounting can fail. All three agree on every file in the corpus. The six that
pass beat their nulls by 1.51 to 4.92 times and their flipped readings by 1.87 to
10.00; unit A loses to its own null by 1.56 and to its own flipped reading by
2.03.

**The margins are not all comfortable, and the thin one has a name.** `1- 8k30p
stable` passes at 3.75 against a null of 5.68 over 39 leaned frames of an 83
second capture, which is the whole of what that file offers. Read at 10 degrees
of lean rather than 8 it offers none at all and goes Blind (measured), and `2
Lens Sharpness Test` falls from 72 leaned frames to 6. The bar sits at 8 for
that reason as much as any other.

### 6.2 What the self-check CANNOT see, and where the mounting is really verified

**HIGH, as arithmetic rather than as a measurement.** The line this check prints
said the mounting was "confirmed" until 2026-08-09. It cannot confirm a mounting,
and the table above says so if you count its columns: there are two distinct
scores across the eight sign families, not eight.

Both of the file's records are carried through the mounting on the way into the
comparison - the quaternion by its reading, the accelerometer by `plumb` - so
anything the mounting does to one it does to the other. The accelerometer's
signs are NOT the reading's own: the shipped constant is `reading = [-1, 1, -1]`
with `plumb = [1, -1, 1]`, i.e. `plumb = s * s2`, because the `S z = s2 z`
factor rides on both sides. For a reading with signs `s`, `S = diag(s)`:

```text
S z = s2 z      predicted = R_s^T z = s2 . S R^T z
up = -diag(s * s2) a / |a| = -s2 . S a / |a|
angle(predicted, up) = angle(S R^T z, -S a/|a|) = angle(R^T z, -a / |a|)
                                       // s2 cancels, then S drops out
```

So the **mirror is invisible** to it, and so is the turn, which rotates both. The
`as written` family in the table of section 6 - refuted from the picture at 65.9
degrees rms of scatter - scores this check identically to the shipped one, to the
last bit, while composing an orientation tens of degrees away.
`osmo::tests::the_check_scores_a_mirrored_mounting_identically` is that as a
test.

**What it does see is real, and there are two things:**

- **A file whose own two records disagree**, which is unit A: 23.0 degrees
  against a 14.7 degree null. That is a fault in the file by the file's own
  evidence, and it needs no reference to any mounting.
- **The conjugation**, which is the one bit of the mounting that does not cancel:
  reading the quaternion forwards rather than inverted transposes the rotation
  and moves the angle. That is the `flipped` column, and it is a bar and not
  only a print, so the verdict rests on the one mounting-sensitive number
  available.

**Mounting verification lives offline**, in the vanishing-point solve of section
6: 23 instants of three unit B files over 177 degrees of lean azimuth, on
rendered pictures, with the lock off. That is where the mirror and the quarter
turn were settled and it is where a second unit's would have to be settled.

**The honest limitation, stated rather than discovered later:** if some other
Osmo 360 writes its inertial frame through a different reflection, that capture
renders a **visibly tilted horizon** and this gate prints the same line it prints
for a file that agrees. Nothing at open would catch it. There is no way to move
that check to open time from gravity alone, because gravity is one direction and
the reflection moves both of the vectors being compared. What would catch it is
the eye, or a re-run of the offline instrument on that unit's own film.

**Unit A fails, and that is the honest outcome rather than a bug.** Its own two
records disagree by 23.0 degrees where a camera assumed upright would be out by
14.7, so Kjerag has no orientation for that file it can believe. It comes out
with an empty orientation track, which is the same shape a capture with no
inertial record at all comes out with, so it reaches the pilot as the disabled
menu item and the same `level:` line, never as an error. Its picture is
untouched, and `lock=0` and `lock=1` render byte-identically there.

What the check does NOT say is WHICH of the two records is the wrong one. It is
worth ruling out the tidiest explanation, though, because the numbers do:
"unit A is simply the other conjugation family" does not hold. Read that way
its miss is its flipped column, 11.3 degrees, which clears its own 14.7 null by
only 1.30 where every passing file clears its own by 1.51 or more, and which is
past the 8 degree ceiling outright. There is no reading of that file's
quaternion, in either family, that agrees with that file's own accelerometer
well enough to hold a horizon on.

**Blind is not refusal.** On a camera that never leans the check cannot separate
one reading from another, and does not need to: the error it exists to catch is
a lean pointed the wrong way, and a file with no lean cannot show one. Those
files hold.

## 7. The honest unknowns

Named so the next reader does not think they were checked.

- **Fields 20 and 27.** **UNKNOWN.** The same two `f32`s as each other in every
  entry of both units, -0.00055 to +0.00064. As radians that is 0.022 to 0.046
  degrees, three orders of magnitude short of a mounting angle. Nothing read here
  can tell them apart, so neither is applied.
- **Field 24, field 25.** **UNKNOWN.** 24 is `8.0` as an `f32` on all four
  entries of both units, so it is not an octant index. 25 is `-1000.0`, on unit
  B's two entries only, absent from unit A's. A constant and a sentinel; neither
  moves with anything measured.
- **The ladder rule.** **UNKNOWN, and it is the one with a size attached.** The
  calibration has 24 slots. Slots 1 and 2 are read, 3 to 10 are empty, and 11 to
  24 are seven pairs walking down about 0.03% a rung. What SELECTS a rung is not
  known - focus distance, temperature and a stabilization crop are all consistent
  with what is seen. Kjerag always reads slot 1. Slot 1 sits about 0.16% above the
  top rung and 0.35% above the bottom, so **up to about a third of a percent of
  scale** is at stake if the rung is ever meant to be chosen.
- **The mounting angle is not written anywhere this audit could read.** It was
  measured from the picture. The one place not looked at is inside the HEVC and
  AAC payloads.
- **D-Log M is out of scope.** Those files play, with the log look. No transform
  for it is in the container, and inventing one is not this branch's job. Two of
  the seven corpus files are D-Log M and they are used here only for their
  telemetry.

## 8. What the seam cannot do here, and why nothing tries

**HIGH, and it is a structural fact rather than a tuning decision.**

- **No inter-lens translation is recorded.** The parallax band needs a baseline
  and there is none, so it switches off. Far-field content joins cleanly; near
  field shows soft doubling at metre range, and that is parallax, not
  calibration. It is the accepted v1.
- **A seam fit cannot move this camera's picture at all.** The fit's knobs write
  `lens.pose`, and a DJI lens takes the `mounting` branch of `lens_from_body`,
  which uses the whole rotation the file records and never consults the pose. So
  the fitter is structurally inert here. It refuses honestly - on both units it
  found too few azimuths it could match and kept the factory calibration - but
  the refusal is not what makes it harmless; the wiring is.

Since #176 the seam applies no band correction to any camera, so the second half
of that is now the general case rather than a DJI one. The first half is not: a
baseline is still what an `.OSV` does not have.
